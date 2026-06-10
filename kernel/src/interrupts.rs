use crate::serial_println;
use core::sync::atomic::{AtomicU64, Ordering};
use pic8259::ChainedPics;
use spin::{Lazy, Mutex};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
const TIMER_VECTOR: u8 = PIC_1_OFFSET; // IRQ0
const KEYBOARD_VECTOR: u8 = PIC_1_OFFSET + 1; // IRQ1

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// PIT ticks since boot, default 18.2 Hz. The cursor blinks off this.
pub static TICKS: AtomicU64 = AtomicU64::new(0);

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
    }
    // Diagnostic handlers so a first exception reports itself instead of
    // escalating to an opaque double fault.
    idt.general_protection_fault.set_handler_fn(gp_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt[TIMER_VECTOR].set_handler_fn(timer_handler);
    idt[KEYBOARD_VECTOR].set_handler_fn(keyboard_handler);
    idt
});

extern "x86-interrupt" fn gp_fault_handler(frame: InterruptStackFrame, code: u64) {
    panic!("GENERAL PROTECTION FAULT (code {:#x}): {:#?}", code, frame);
}

extern "x86-interrupt" fn page_fault_handler(
    frame: InterruptStackFrame,
    code: x86_64::structures::idt::PageFaultErrorCode,
) {
    let cr2 = x86_64::registers::control::Cr2::read();
    panic!("PAGE FAULT at {:?} (code {:?}): {:#?}", cr2, code, frame);
}

extern "x86-interrupt" fn invalid_opcode_handler(frame: InterruptStackFrame) {
    panic!("INVALID OPCODE: {:#?}", frame);
}

extern "x86-interrupt" fn breakpoint_handler(frame: InterruptStackFrame) {
    serial_println!("BREAKPOINT: {:#?}", frame);
}

pub fn init() {
    crate::gdt::init();
    IDT.load();
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // Unmask only IRQ0 (timer) and IRQ1 (keyboard).
        pics.write_masks(0b1111_1100, 0b1111_1111);
    }
    // Drain the i8042 output buffer so a stale firmware byte can't wedge IRQ1.
    crate::keyboard::init();
    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn timer_handler(_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe { PICS.lock().notify_end_of_interrupt(TIMER_VECTOR) };
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let scancode: u8 = unsafe { Port::new(0x60).read() };
    serial_println!("IRQ1 sc={:#x}", scancode);
    crate::keyboard::push(scancode);
    unsafe { PICS.lock().notify_end_of_interrupt(KEYBOARD_VECTOR) };
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("DOUBLE FAULT ({}): {:#?}", code, frame);
}
