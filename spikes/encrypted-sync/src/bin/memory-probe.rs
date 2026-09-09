//! Instrumented allocator observes PUBLIC canaries before deallocation; no freed-memory reads.
use automerge::{Automerge, ROOT, transaction::Transactable};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
static OBSERVE: AtomicBool = AtomicBool::new(false);
static FOUND: AtomicUsize = AtomicUsize::new(0);
const MARKER: &[u8] = b"PUBLIC_MEMORY_CANARY_NOT_A_REAL_SECRET_123456789";
struct Observer;
// SAFETY: scanning happens over a live allocation immediately before System.dealloc.
// Object representations are inspected by C memmem, never after deallocation.
unsafe impl GlobalAlloc for Observer {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if OBSERVE.load(Ordering::Relaxed) {
            // SAFETY: memmem reads the live object representation as C bytes,
            // including possible padding; no Rust slice of uninitialized u8 is made.
            let found =
                unsafe { libc::memmem(p.cast(), l.size(), MARKER.as_ptr().cast(), MARKER.len()) };
            if !found.is_null() {
                FOUND.fetch_add(1, Ordering::Relaxed);
            }
        }
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align(n, l.align()).unwrap();
        let new = unsafe { System.alloc_zeroed(new_layout) };
        if !new.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(p, new, l.size().min(n));
                self.dealloc(p, l);
            }
        }
        new
    }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
fn main() {
    let protected = zeroize::Zeroizing::new(MARKER.to_vec());
    OBSERVE.store(true, Ordering::SeqCst);
    drop(protected);
    OBSERVE.store(false, Ordering::SeqCst);
    let protected_frees = FOUND.swap(0, Ordering::SeqCst);
    let mut doc = Automerge::new();
    let mut tx = doc.transaction();
    tx.put(ROOT, "field", std::str::from_utf8(MARKER).unwrap())
        .unwrap();
    tx.commit();
    OBSERVE.store(true, Ordering::SeqCst);
    drop(doc);
    OBSERVE.store(false, Ordering::SeqCst);
    let automerge_frees = FOUND.load(Ordering::SeqCst);
    println!(
        "zeroizing_uncleared_frees={protected_frees} automerge_uncleared_frees={automerge_frees}"
    );
    // Nonzero is a recorded failed security criterion, not a successful cleanup test.
    if protected_frees != 0 || automerge_frees != 0 {
        std::process::exit(2);
    }
}
