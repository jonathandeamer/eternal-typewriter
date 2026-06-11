use spin::Mutex;
use x86_64::instructions::port::Port;

const STATUS_OUTPUT_FULL: u8 = 1 << 0; // 0x64 bit 0: data waiting in 0x60
const STATUS_INPUT_FULL: u8 = 1 << 1; // 0x64 bit 1: controller still busy

/// Spin until the controller's input buffer is empty so it can accept a byte.
fn wait_writable(status: &mut Port<u8>) {
    for _ in 0..100_000 {
        if unsafe { status.read() } & STATUS_INPUT_FULL == 0 {
            return;
        }
    }
}

/// Bring the i8042 keyboard up explicitly. SeaBIOS does not reliably leave
/// IRQ1 enabled or scanning on under QEMU, so without this the controller
/// never asserts a keyboard interrupt. Drain the output buffer, set the
/// controller config byte (IRQ1 on, keyboard clock on, translation kept so
/// we keep scancode set 1), then tell the keyboard to start scanning.
pub fn init() {
    let mut status: Port<u8> = Port::new(0x64);
    let mut data: Port<u8> = Port::new(0x60);
    unsafe {
        // Drain anything firmware left behind, else a stale byte wedges IRQ1.
        for _ in 0..32 {
            if status.read() & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let _ = data.read();
        }
        // Read the controller configuration byte (command 0x20).
        wait_writable(&mut status);
        status.write(0x20u8);
        for _ in 0..100_000 {
            if status.read() & STATUS_OUTPUT_FULL != 0 {
                break;
            }
        }
        let mut config = data.read();
        config |= 1 << 0; // enable first-port (keyboard) interrupt (IRQ1)
        config &= !(1 << 4); // enable first-port clock (0 = enabled)
        // Write the controller configuration byte back (command 0x60).
        wait_writable(&mut status);
        status.write(0x60u8);
        wait_writable(&mut status);
        data.write(config);
        // Enable the keyboard port at the controller (command 0xAE).
        wait_writable(&mut status);
        status.write(0xAEu8);
        // Tell the keyboard device to enable scanning (0xF4); it replies 0xFA.
        wait_writable(&mut status);
        data.write(0xF4u8);
        for _ in 0..100_000 {
            if status.read() & STATUS_OUTPUT_FULL != 0 {
                let _ = data.read(); // consume the ACK
                break;
            }
        }
    }
}

/// Fixed ring so the IRQ handler never allocates. 64 pending scancodes is
/// far beyond human typing speed; overflow drops the newest input.
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
