//! Per-script memory quotas: an allocator that charges each allocation to the script that
//! is running when it is made, and refuses the ones that would take it past its quota.
//!
//! QuickJS accounts memory per runtime, not per context, so the host does it: the engine
//! names the running script ([`Quota::charge_to`]) around everything it runs for it, and
//! this allocator, which sees every allocation of the runtime, keeps a count for it.
//! Each block records its owner in its header, so a block is credited back to the script
//! that paid for it, whoever frees it (the garbage collector, another script's run).
//!
//! What it does not do: split what scripts share. An allocation made while no script is
//! named (the engine's own, the first compile of the prelude that every context uses) is
//! charged to nobody, and the runtime's own limit still bounds the sum.

use std::alloc::{self, Layout};
use std::cell::{Cell, RefCell};
use std::mem;
use std::ptr;
use std::rc::Rc;

use rquickjs::allocator::Allocator;

/// QuickJS allocates values of up to 8 bytes of alignment.
const ALIGN: usize = mem::align_of::<u64>();
/// One word before each block: its size in the low bits, its owner in the high ones.
const HEADER: usize = mem::size_of::<u64>();
const SIZE_BITS: u32 = 40;
const SIZE_MASK: u64 = (1 << SIZE_BITS) - 1;
/// The most scripts that can hold a quota at once, with the owner of nobody being 0.
const MAX_OWNERS: usize = (1 << (64 - SIZE_BITS)) - 1;

/// What one script is charged, and whether it was refused since last asked.
struct Account {
    used: Cell<usize>,
    /// An allocation was refused for being over the quota.
    refused: Cell<bool>,
    /// Its script is gone: the slot is free once what it paid for is freed.
    retired: Cell<bool>,
}

/// The books of an engine's scripts, shared by the engine and its allocator.
pub(super) struct Quota {
    limit: usize,
    /// The owner of the allocations being made: an id from 1, or 0 for nobody.
    current: Cell<u32>,
    accounts: RefCell<Vec<Account>>,
}

impl Quota {
    pub(super) fn new(limit: usize) -> Rc<Self> {
        Rc::new(Self {
            limit,
            current: Cell::new(0),
            accounts: RefCell::new(Vec::new()),
        })
    }

    /// The most bytes one script may hold.
    pub(super) fn limit(&self) -> usize {
        self.limit
    }

    /// A new owner, reusing the slot of a script that is gone and holds nothing.
    pub(super) fn new_owner(self: &Rc<Self>) -> Owner {
        let mut accounts = self.accounts.borrow_mut();
        let free = accounts
            .iter()
            .position(|account| account.retired.get() && account.used.get() == 0);
        let index = free.unwrap_or_else(|| {
            assert!(
                accounts.len() < MAX_OWNERS,
                "an engine holds at most {MAX_OWNERS} script owners"
            );
            accounts.push(Account {
                used: Cell::new(0),
                refused: Cell::new(false),
                retired: Cell::new(false),
            });
            accounts.len() - 1
        });
        let account = &accounts[index];
        account.retired.set(false);
        account.refused.set(false);
        // Ids start at 1 (0 means no owner) and the cap keeps them within the header's owner bits.
        let id = u32::try_from(index + 1).expect("owner id fits the cap");
        Owner {
            quota: Rc::clone(self),
            id,
        }
    }

    /// Charges the allocations made until the guard drops to `owner`, and forgets that it
    /// was refused before.
    pub(super) fn charge_to(&self, owner: &Owner) -> ChargeGuard<'_> {
        self.accounts.borrow()[owner.index()].refused.set(false);
        ChargeGuard {
            quota: self,
            previous: self.current.replace(owner.id),
        }
    }

    fn charge(&self, owner: u32, bytes: usize, already_held: usize) -> bool {
        let accounts = self.accounts.borrow();
        let account = &accounts[owner as usize - 1];
        let used = account.used.get();
        if used + bytes.saturating_sub(already_held) > self.limit {
            account.refused.set(true);
            return false;
        }
        account.used.set(used + bytes);
        true
    }

    /// Gives back `bytes` charged to `owner`. Every block is refunded once, to the owner
    /// it was charged to, and an account is reused only once it holds nothing, so `bytes`
    /// never exceeds what `owner` holds.
    fn credit(&self, owner: u32, bytes: usize) {
        let accounts = self.accounts.borrow();
        let account = &accounts[owner as usize - 1];
        let used = account.used.get();
        debug_assert!(bytes <= used, "refund exceeds the owner's charge");
        account.used.set(used - bytes);
    }
}

/// A script's seat in the books. Dropping it retires the account: the script's memory is
/// still counted until it is freed.
pub(super) struct Owner {
    quota: Rc<Quota>,
    id: u32,
}

impl Owner {
    fn index(&self) -> usize {
        self.id as usize - 1
    }

    /// Bytes the script holds right now.
    pub(super) fn used(&self) -> usize {
        self.quota.accounts.borrow()[self.index()].used.get()
    }

    /// Whether an allocation of the script was refused for being over its quota since it
    /// was last charged to, which this clears.
    pub(super) fn take_refused(&self) -> bool {
        self.quota.accounts.borrow()[self.index()]
            .refused
            .replace(false)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.quota.accounts.borrow()[self.index()].retired.set(true);
    }
}

/// Names the script that allocations are charged to, until it drops.
pub(super) struct ChargeGuard<'a> {
    quota: &'a Quota,
    previous: u32,
}

impl Drop for ChargeGuard<'_> {
    fn drop(&mut self) {
        self.quota.current.set(self.previous);
    }
}

/// The Rust global allocator, charging a [`Quota`].
pub(super) struct QuotaAllocator {
    quota: Rc<Quota>,
}

impl QuotaAllocator {
    pub(super) fn new(quota: Rc<Quota>) -> Self {
        Self { quota }
    }

    /// Reserves a block of `size` bytes (already rounded) for the current owner, writing
    /// its header; null when the owner is over its quota, the size is beyond what a
    /// header records, or the allocator fails.
    fn allocate(&self, size: usize, zeroed: bool) -> *mut u8 {
        if size > SIZE_MASK as usize {
            return ptr::null_mut();
        }
        let owner = self.quota.current.get();
        if owner != 0 && !self.quota.charge(owner, size, 0) {
            return ptr::null_mut();
        }
        let Ok(layout) = Layout::from_size_align(HEADER + size, ALIGN) else {
            self.refund(owner, size);
            return ptr::null_mut();
        };
        // SAFETY: the layout has a non-zero size, since it includes the header.
        let block = unsafe {
            if zeroed {
                alloc::alloc_zeroed(layout)
            } else {
                alloc::alloc(layout)
            }
        };
        if block.is_null() {
            self.refund(owner, size);
            return ptr::null_mut();
        }
        // SAFETY: `block` is `HEADER + size` bytes and aligned for a `u64`.
        unsafe {
            block.cast::<u64>().write(header(size, owner));
            block.add(HEADER)
        }
    }

    fn refund(&self, owner: u32, size: usize) {
        if owner != 0 {
            self.quota.credit(owner, size);
        }
    }
}

fn round(size: usize) -> usize {
    size.div_ceil(ALIGN) * ALIGN
}

fn header(size: usize, owner: u32) -> u64 {
    debug_assert!(size as u64 <= SIZE_MASK);
    size as u64 | (u64::from(owner) << SIZE_BITS)
}

/// The layout of a block of `size` payload bytes, header included.
///
/// # Safety
/// `size` must be a size this allocator accepted, so at most `SIZE_MASK`; the total is
/// then a valid layout.
unsafe fn block_layout(size: usize) -> Layout {
    debug_assert!(size as u64 <= SIZE_MASK);
    // SAFETY: `HEADER + size` is below 2^40 + 8, and `ALIGN` is a power of two.
    unsafe { Layout::from_size_align_unchecked(HEADER + size, ALIGN) }
}

/// Reads the size and the owner recorded before `ptr`.
///
/// # Safety
/// `ptr` must be a pointer this allocator returned: a `u64` header written by [`header`]
/// sits `HEADER` bytes before it.
unsafe fn read_header(ptr: *mut u8) -> (usize, u32) {
    // SAFETY: the caller guarantees a `u64` header precedes `ptr`.
    let word = unsafe { ptr.sub(HEADER).cast::<u64>().read() };
    ((word & SIZE_MASK) as usize, (word >> SIZE_BITS) as u32)
}

// SAFETY: blocks are at least `size` bytes, 8-aligned, and `usable_size` reports the size
// recorded in the header, as the Rust allocator of rquickjs does.
unsafe impl Allocator for QuotaAllocator {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        self.allocate(round(size), false)
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        if count == 0 || size == 0 {
            return ptr::null_mut();
        }
        match count.checked_mul(size) {
            Some(total) => self.allocate(round(total), true),
            None => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        // SAFETY: `ptr` came from this allocator, so a header precedes it and the block
        // was allocated with this layout.
        unsafe {
            let (size, owner) = read_header(ptr);
            self.refund(owner, size);
            let layout = block_layout(size);
            alloc::dealloc(ptr.sub(HEADER), layout);
        }
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            return self.allocate(round(new_size), false);
        }
        let new_size = round(new_size);
        if new_size > SIZE_MASK as usize {
            return ptr::null_mut();
        }
        // SAFETY: `ptr` came from this allocator.
        let (old_size, old_owner) = unsafe { read_header(ptr) };
        let owner = self.quota.current.get();
        if owner != 0 {
            // A block the running script already holds only costs the growth. The whole
            // new size is charged, and the old one credited once the move succeeded.
            let held = if owner == old_owner { old_size } else { 0 };
            if !self.quota.charge(owner, new_size, held) {
                return ptr::null_mut();
            }
        }
        // SAFETY: the old block was allocated with this layout, and `new_size` is below
        // 2^40, so the new layout cannot overflow.
        let block = unsafe {
            let layout = block_layout(old_size);
            alloc::realloc(ptr.sub(HEADER), layout, HEADER + new_size)
        };
        if block.is_null() {
            // The old block stands: undo what was charged for the new one.
            self.refund(owner, new_size);
            return ptr::null_mut();
        }
        self.refund(old_owner, old_size);
        // SAFETY: `block` is `HEADER + new_size` bytes and aligned for a `u64`.
        unsafe {
            block.cast::<u64>().write(header(new_size, owner));
            block.add(HEADER)
        }
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        // SAFETY: `ptr` came from this allocator.
        unsafe { read_header(ptr).0 }
    }
}
