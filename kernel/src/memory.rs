use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::structures::paging::{
    FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

/// Safety: physical memory must be fully mapped at `physical_memory_offset`
/// (the bootloader config requests this) and this must be called once.
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let (frame, _) = x86_64::registers::control::Cr3::read();
    let virt = physical_memory_offset + frame.start_address().as_u64();
    let table: *mut PageTable = virt.as_mut_ptr();
    OffsetPageTable::new(&mut *table, physical_memory_offset)
}

/// Hands out usable physical frames in order. A persistent (region, address)
/// cursor keeps `allocate_frame` O(1): the naive blog_os version rescans with
/// `usable_frames().nth(next)` on every call, which is O(n²) and takes minutes
/// to map a 256 MiB heap (65,536 frames). This keeps boot under a second.
pub struct BootInfoFrameAllocator {
    memory_regions: &'static MemoryRegions,
    region_idx: usize,
    next_addr: u64, // next physical address to consider within the current region
}

impl BootInfoFrameAllocator {
    /// Safety: all `Usable` regions must really be unused.
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        BootInfoFrameAllocator { memory_regions, region_idx: 0, next_addr: 0 }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        while self.region_idx < self.memory_regions.len() {
            let region = self.memory_regions[self.region_idx];
            if region.kind != MemoryRegionKind::Usable {
                self.region_idx += 1;
                self.next_addr = 0;
                continue;
            }
            // Align the cursor up to a frame boundary within this region.
            let start = self.next_addr.max(region.start);
            let frame_addr = (start + 4095) & !4095;
            if frame_addr + 4096 <= region.end {
                self.next_addr = frame_addr + 4096;
                return Some(PhysFrame::containing_address(PhysAddr::new(frame_addr)));
            }
            // Region exhausted; advance to the next one.
            self.region_idx += 1;
            self.next_addr = 0;
        }
        None
    }
}
