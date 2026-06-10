use spin::Lazy;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{Segment, CS, SS};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
        const STACK_SIZE: usize = 4096 * 5;
        static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let start = VirtAddr::from_ptr(&raw const STACK);
        start + STACK_SIZE as u64
    };
    tss
});

static GDT: Lazy<(GlobalDescriptorTable, SegmentSelector, SegmentSelector)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code = gdt.append(Descriptor::kernel_code_segment());
    let tss_sel = gdt.append(Descriptor::tss_segment(&TSS));
    (gdt, code, tss_sel)
});

pub fn init() {
    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1);
        // The bootloader leaves SS pointing into its own GDT (selector 0x10),
        // which in our GDT is the TSS descriptor — an invalid data segment.
        // Long mode ignores SS for addressing, but iretq reloads it on the
        // first interrupt return and #GPs on the stale selector. Reset SS to
        // the null selector, which is valid at ring 0 in long mode.
        SS::set_reg(SegmentSelector::NULL);
        load_tss(GDT.2);
    }
}
