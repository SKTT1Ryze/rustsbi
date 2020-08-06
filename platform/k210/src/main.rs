#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(global_asm)]
#![feature(llvm_asm)]

use core::alloc::Layout;
use core::panic::PanicInfo;
use k210_hal::{clock::Clocks, fpioa, pac, prelude::*};
use linked_list_allocator::LockedHeap;
use rustsbi::{enter_privileged, println};
use riscv::register::{
    mepc, mhartid,
    mstatus::{self, MPP},
};

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[alloc_error_handler]
fn oom(_layout: Layout) -> ! {
    loop {}
}


#[export_name = "_start"]
#[link_section = ".text.entry"] // this is stable
fn main() -> ! {
    unsafe {
        llvm_asm!(
            "
        csrr    a2, mhartid
        lui     t0, %hi(_max_hart_id)
        add     t0, t0, %lo(_max_hart_id)
        bgtu    a2, t0, _start_abort
        la      sp, _stack_start
        lui     t0, %hi(_hart_stack_size)
        add     t0, t0, %lo(_hart_stack_size)
    .ifdef __riscv_mul
        mul     t0, a2, t0
    .else
        beqz    a2, 2f  // Jump if single-hart
        mv      t1, a2
        mv      t2, t0
    1:
        add     t0, t0, t2
        addi    t1, t1, -1
        bnez    t1, 1b
    2:
    .endif
        sub     sp, sp, t0
        csrw    mscratch, zero
        j _start_success
        
    _start_abort:
        wfi
        j _start_abort
    _start_success:
        
    "
        )
    };
    if mhartid::read() == 0 {
        extern "C" {
            fn _sheap();
            fn _heap_size();
        }
        let sheap = &mut _sheap as *mut _ as usize;
        let heap_size = &_heap_size as *const _ as usize;
        unsafe {
            ALLOCATOR.lock().init(sheap, heap_size);
        }

        let p = pac::Peripherals::take().unwrap();

        let mut sysctl = p.SYSCTL.constrain();
        let fpioa = p.FPIOA.split(&mut sysctl.apb0);
        let clocks = Clocks::new();
        let _uarths_tx = fpioa.io5.into_function(fpioa::UARTHS_TX);
        let _uarths_rx = fpioa.io4.into_function(fpioa::UARTHS_RX);
        // Configure UART
        let serial = p.UARTHS.configure(115_200.bps(), &clocks);
        let (tx, rx) = serial.split();
        use rustsbi::legacy_stdio::init_legacy_stdio_embedded_hal_fuse;
        init_legacy_stdio_embedded_hal_fuse(tx, rx);

        println!("[rustsbi] Version 0.1.0");

        println!("{}", rustsbi::LOGO);
        println!("[rustsbi] Target device: K210");
        println!("[rustsbi] Kernel entry: 0x80200000");
    }
    extern "C" {
        fn _s_mode_start();
    }
    unsafe {
        mepc::write(_s_mode_start as usize);
        mstatus::set_mpp(MPP::Supervisor);
        enter_privileged(mhartid::read(), 0x2333333366666666);
    }
}

global_asm!(
    "
    .section .text
    .globl _s_mode_start
_s_mode_start:
1:  auipc ra, %pcrel_hi(1f)
    ld ra, %pcrel_lo(1b)(ra)
    jr ra
.align  3
1:  .dword 0x80200000
"
);

// todo: configurable target address