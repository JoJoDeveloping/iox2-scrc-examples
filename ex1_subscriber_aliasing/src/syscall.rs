use std::arch::asm;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct PayloadData {
    x: i32,
}

/// A compiler barrier. This has no effects, but inhibits compiler optimization.
/// More specifically, this function contains a (no-op) `asm!` block to which any
/// "story code" can be attached.
#[inline(always)]
fn compiler_barrier() {
    // SAFETY: This does not actually do anything.
    unsafe { 
        asm!("")
    }
}

fn main() -> Result<(), Box<dyn core::error::Error>> {
    let shm_name = c"/hypnotoad";
    let shm_size = 4096 * 10;
    let offset = 128;
    let payload = PayloadData { x: 123098 };

    // In Process 1: create new shared memory (publisher)
    // `Publisher::create()`
    let shm_fd_publisher =
        unsafe { libc::shm_open(shm_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o666) };
    unsafe { libc::ftruncate(shm_fd_publisher, shm_size) };
    // STORY CODE:
    // The `mmap` syscall creates a non-read-only Rust AM allocation of size `shm_size`, backed by the shared memory object.
    // It is basically like calling Box::new_uninit.
    // It also spawns a new story thread, that for now does nothing (i.e. it waits for a command queue to be filled). Call this thread "A"
    // STORY CODE SAFETY:
    // We assume that we're for now the only process using this specific shared memory object, i.e. no other process races
    // with what we do to it.
    let shm_base_publisher = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            shm_size as _,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            shm_fd_publisher,
            0,
        )
    };

    // In Process 2: open data segment of publisher (subscriber 1)
    // `Subscriber::open()`
    let shm_fd_subscriber_1 =
        unsafe { libc::shm_open(shm_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o666) };
    // STORY CODE:
    // This mmap creates a new allocation, similar to the above. But it spawns a background thread (called "B") that continuously
    // writes (non-atomically) to all bytes in that allocation, making it UB to access the bytes in this allocation in any way.
    // SAFETY: We do not (for now) access memory through this pointer
    let shm_base_subscriber_1 = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            shm_size as _,
            libc::PROT_READ,
            libc::MAP_SHARED,
            shm_fd_subscriber_1,
            0,
        )
    };

    // In Process 3: open data segment of publisher (subscriber 2)
    let shm_fd_subscriber_2 =
        unsafe { libc::shm_open(shm_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o666) };
    // STORY CODE: like above but call the thread "C"
    // SAFETY: like above
    let shm_base_subscriber_2 = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            shm_size as _,
            libc::PROT_READ,
            libc::MAP_SHARED,
            shm_fd_subscriber_2,
            0,
        )
    };

    // Publisher writing data:
    // `Publisher::loan_uninit()`
    let sample_mut: *mut PayloadData =
        unsafe { shm_base_publisher.add(offset).cast::<PayloadData>() };

    // `SampleMutUninit::write_payload()`
    unsafe { sample_mut.write(payload) };

    // `SampleMut::send()` writes the offset to the payload in an internal lock-free queue
    // read-only sample is still available on sender side
    // We don't need to do any of the actual IPC/lock-free queue manipulation in this example, but we need a compiler barrier.
    // In the actual implementation of `SampleMut::send` we might have to insert this barrier.
    // STORY CODE:
    // This dispatches thread "A" spawned above to read (non-atomically) the shared memory written to so far, in a loop.
    // This will happen continously and makes those bytes effectively read-only.
    // SAFETY:
    // We do not read from `shm_base_publisher` after this call returns.
    compiler_barrier();

    let sample: *const PayloadData = sample_mut.cast_const();
    drop(sample_mut);
    println!("send payload: {payload:?}");

    // Subscribers receiving data:
    // `Subscriber_1::receive()`;
    // Again, we do a compiler barrier, this time on the recieving side. Note that doing two barriers in a row is of course silly in practice;
    // we do it here to separate concerns between reader and writer which would usually be in separate threads/programs.
    // STORY CODE:
    // we make threads "B" and "C" copy (non-atomically) the bytes just made read-only above from the allocation backing `shm_base_publisher` to the one backing `shm_base_subscriber_N`.
    // Afterwards, the threads stop writing to these bytes and only read (non-atomically), which effectively makes these bytes read-only (instead of not being accessible at all).
    // SAFETY:
    // We do not write to `shm_base_subscriber_N` after this call returns.
    compiler_barrier();


    let sample_1: *const PayloadData =
        unsafe { shm_base_subscriber_1.add(offset).cast::<PayloadData>() };

    println!("subscriber 1 received: {:?}", unsafe { &*sample_1 });

    // `Subscriber_2::receive()`;
    // STORY CODE is folded into the above, as is SAFETY.
    let sample_2: *const PayloadData =
        unsafe { shm_base_subscriber_2.add(offset).cast::<PayloadData>() };
    println!("subscriber 2 received: {:?}", unsafe { &*sample_2 });

    // STORY CODE: we stop the three threads "A", "B", and "C", and deallocate the three blocks.
    // SAFETY: we do not access these memory regions afterwards.
    unsafe { libc::shm_unlink(shm_name.as_ptr()) };

    Ok(())
}
