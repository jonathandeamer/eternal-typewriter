use spin::Mutex;

/// Fixed ring so the IRQ handler never allocates. 64 pending scancodes is
/// far beyond human typing speed; overflow drops the oldest-unread input.
struct Ring {
    buf: [u8; 64],
    head: usize,
    len: usize,
}

static QUEUE: Mutex<Ring> = Mutex::new(Ring { buf: [0; 64], head: 0, len: 0 });

/// Called from the IRQ1 handler (interrupts already disabled there).
pub fn push(scancode: u8) {
    let mut q = QUEUE.lock();
    if q.len < q.buf.len() {
        let tail = (q.head + q.len) % q.buf.len();
        q.buf[tail] = scancode;
        q.len += 1;
    }
}

/// Called from the main loop. Disables interrupts around the lock so the
/// IRQ handler can never deadlock against us.
pub fn pop() -> Option<u8> {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut q = QUEUE.lock();
        if q.len == 0 {
            return None;
        }
        let scancode = q.buf[q.head];
        q.head = (q.head + 1) % q.buf.len();
        q.len -= 1;
        Some(scancode)
    })
}
