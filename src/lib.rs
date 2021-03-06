//! A lock-free, eventually consistent, concurrent multi-value map.
//!
//! This map implementation allows reads and writes to execute entirely in parallel, with no
//! implicit synchronization overhead. Reads never take locks on their critical path, and neither
//! do writes assuming there is a single writer (multi-writer is possible using a `Mutex`), which
//! significantly improves performance under contention. See the [`left-right` crate](left_right)
//! for details on the underlying concurrency primitive.
//!
//! The trade-off exposed by this type is one of eventual consistency: writes are not visible to
//! readers except following explicit synchronization. Specifically, readers only see the
//! operations that preceeded the last call to `WriteHandle::refresh` by a writer. This lets
//! writers decide how stale they are willing to let reads get. They can refresh the map after
//! every write to emulate a regular concurrent `HashMap`, or they can refresh only occasionally to
//! reduce the synchronization overhead at the cost of stale reads.
//!
//! For read-heavy workloads, the scheme used by this module is particularly useful. Writers can
//! afford to refresh after every write, which provides up-to-date reads, and readers remain fast
//! as they do not need to ever take locks.
//!
//! The map is multi-value, meaning that every key maps to a *collection* of values. This
//! introduces some memory cost by adding a layer of indirection through a `Vec` for each value,
//! but enables more advanced use. This choice was made as it would not be possible to emulate such
//! functionality on top of the semantics of this map (think about it -- what would the operational
//! log contain?).
//!
//! To faciliate more advanced use-cases, each of the two maps also carry some customizeable
//! meta-information. The writers may update this at will, and when a refresh happens, the current
//! meta will also be made visible to readers. This could be useful, for example, to indicate what
//! time the refresh happened.
//!
//! # Features
//!
//!  - `eviction`: Gives you access to [`WriteHandle::empty_random`] to empty out randomly chosen
//!    keys from the map.
//!  - `amortize`: Amortizes the cost of resizes in the underlying data structures. See
//!    [`griddle`](https://github.com/jonhoo/griddle/) and
//!    [`atone`](https://github.com/jonhoo/atone/) for details. This requires a nightly compiler
//!    [for the time being](https://docs.rs/indexmap-amortized/1.0/indexmap_amortized/#rust-version).
//!
//!
//! # Examples
//!
//! Single-reader, single-writer
//!
//! ```
//! // new will use the default HashMap hasher, and a meta of ()
//! // note that we get separate read and write handles
//! // the read handle can be cloned to have more readers
//! let (mut book_reviews_w, book_reviews_r) = evmap::new();
//!
//! // review some books.
//! book_reviews_w.insert("Adventures of Huckleberry Finn",    "My favorite book.");
//! book_reviews_w.insert("Grimms' Fairy Tales",               "Masterpiece.");
//! book_reviews_w.insert("Pride and Prejudice",               "Very enjoyable.");
//! book_reviews_w.insert("The Adventures of Sherlock Holmes", "Eye lyked it alot.");
//!
//! // at this point, reads from book_reviews_r will not see any of the reviews!
//! assert_eq!(book_reviews_r.len(), 0);
//! // we need to refresh first to make the writes visible
//! book_reviews_w.publish();
//! assert_eq!(book_reviews_r.len(), 4);
//! // reads will now return Some() because the map has been initialized
//! assert_eq!(book_reviews_r.get("Grimms' Fairy Tales").map(|rs| rs.len()), Some(1));
//!
//! // remember, this is a multi-value map, so we can have many reviews
//! book_reviews_w.insert("Grimms' Fairy Tales",               "Eh, the title seemed weird.");
//! book_reviews_w.insert("Pride and Prejudice",               "Too many words.");
//!
//! // but again, new writes are not yet visible
//! assert_eq!(book_reviews_r.get("Grimms' Fairy Tales").map(|rs| rs.len()), Some(1));
//!
//! // we need to refresh first
//! book_reviews_w.publish();
//! assert_eq!(book_reviews_r.get("Grimms' Fairy Tales").map(|rs| rs.len()), Some(2));
//!
//! // oops, this review has a lot of spelling mistakes, let's delete it.
//! // remove_entry deletes *all* reviews (though in this case, just one)
//! book_reviews_w.remove_entry("The Adventures of Sherlock Holmes");
//! // but again, it's not visible to readers until we refresh
//! assert_eq!(book_reviews_r.get("The Adventures of Sherlock Holmes").map(|rs| rs.len()), Some(1));
//! book_reviews_w.publish();
//! assert_eq!(book_reviews_r.get("The Adventures of Sherlock Holmes").map(|rs| rs.len()), None);
//!
//! // look up the values associated with some keys.
//! let to_find = ["Pride and Prejudice", "Alice's Adventure in Wonderland"];
//! for book in &to_find {
//!     if let Some(reviews) = book_reviews_r.get(book) {
//!         for review in &*reviews {
//!             println!("{}: {}", book, review);
//!         }
//!     } else {
//!         println!("{} is unreviewed.", book);
//!     }
//! }
//!
//! // iterate over everything.
//! for (book, reviews) in &book_reviews_r.enter().unwrap() {
//!     for review in reviews {
//!         println!("{}: \"{}\"", book, review);
//!     }
//! }
//! ```
//!
//! Reads from multiple threads are possible by cloning the `ReadHandle`.
//!
//! ```
//! use std::thread;
//! let (mut book_reviews_w, book_reviews_r) = evmap::new();
//!
//! // start some readers
//! let readers: Vec<_> = (0..4).map(|_| {
//!     let r = book_reviews_r.clone();
//!     thread::spawn(move || {
//!         loop {
//!             let l = r.len();
//!             if l == 0 {
//!                 thread::yield_now();
//!             } else {
//!                 // the reader will either see all the reviews,
//!                 // or none of them, since refresh() is atomic.
//!                 assert_eq!(l, 4);
//!                 break;
//!             }
//!         }
//!     })
//! }).collect();
//!
//! // do some writes
//! book_reviews_w.insert("Adventures of Huckleberry Finn",    "My favorite book.");
//! book_reviews_w.insert("Grimms' Fairy Tales",               "Masterpiece.");
//! book_reviews_w.insert("Pride and Prejudice",               "Very enjoyable.");
//! book_reviews_w.insert("The Adventures of Sherlock Holmes", "Eye lyked it alot.");
//! // expose the writes
//! book_reviews_w.publish();
//!
//! // you can read through the write handle
//! assert_eq!(book_reviews_w.len(), 4);
//!
//! // the original read handle still works too
//! assert_eq!(book_reviews_r.len(), 4);
//!
//! // all the threads should eventually see .len() == 4
//! for r in readers.into_iter() {
//!     assert!(r.join().is_ok());
//! }
//! ```
//!
//! If multiple writers are needed, the `WriteHandle` must be protected by a `Mutex`.
//!
//! ```
//! use std::thread;
//! use std::sync::{Arc, Mutex};
//! let (mut book_reviews_w, book_reviews_r) = evmap::new();
//!
//! // start some writers.
//! // since evmap does not support concurrent writes, we need
//! // to protect the write handle by a mutex.
//! let w = Arc::new(Mutex::new(book_reviews_w));
//! let writers: Vec<_> = (0..4).map(|i| {
//!     let w = w.clone();
//!     thread::spawn(move || {
//!         let mut w = w.lock().unwrap();
//!         w.insert(i, true);
//!         w.publish();
//!     })
//! }).collect();
//!
//! // eventually we should see all the writes
//! while book_reviews_r.len() < 4 { thread::yield_now(); };
//!
//! // all the threads should eventually finish writing
//! for w in writers.into_iter() {
//!     assert!(w.join().is_ok());
//! }
//! ```
//!
//! [`ReadHandle`] is not `Sync` as sharing a single instance amongst threads would introduce a
//! significant performance bottleneck. A fresh `ReadHandle` needs to be created for each thread
//! either by cloning a [`ReadHandle`] or from a [`handles::ReadHandleFactory`]. For further
//! information, see [`left_right::ReadHandle`].
//!
//! # Implementation
//!
//! Under the hood, the map is implemented using two regular `HashMap`s and some magic. Take a look
//! at [`left-right`](left_right) for a much more in-depth discussion. Since the implementation
//! uses regular `HashMap`s under the hood, table resizing is fully supported. It does, however,
//! also mean that the memory usage of this implementation is approximately twice of that of a
//! regular `HashMap`, and more if writers rarely refresh after writing.
//!
//! # Value storage
//!
//! The values for each key in the map are stored in [`refs::Values`]. Conceptually, each `Values`
//! is a _bag_ or _multiset_; it can store multiple copies of the same value. `evmap` applies some
//! cleverness in an attempt to reduce unnecessary allocations and keep the cost of operations on
//! even large value-bags small. For small bags, `Values` uses the `smallvec` crate. This avoids
//! allocation entirely for single-element bags, and uses a `Vec` if the bag is relatively small.
//! For large bags, `Values` uses the `hashbag` crate, which enables `evmap` to efficiently look up
//! and remove specific elements in the value bag. For bags larger than one element, but smaller
//! than the threshold for moving to `hashbag`, we use `smallvec` to avoid unnecessary hashing.
//! Operations such as `Fit` and `Replace` will automatically switch back to the inline storage if
//! possible. This is ideal for maps that mostly use one element per key, as it can improvate
//! memory locality with less indirection.
#![warn(
    missing_docs,
    rust_2018_idioms,
    missing_debug_implementations,
    broken_intra_doc_links
)]
#![allow(clippy::type_complexity)]
// This _should_ detect if we ever accidentally leak aliasing::NoDrop.
// But, currently, it does not..
#![deny(unreachable_pub)]
#![cfg_attr(docsrs, feature(doc_cfg))]

use crate::inner::Inner;
use crate::read::ReadHandle;
use crate::write::WriteHandle;
use left_right::aliasing::Aliased;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hash};

mod inner;
mod read;
mod values;
mod write;

/// Handles to the read and write halves of an `evmap`.
pub mod handles {
    pub use crate::write::WriteHandle;

    // These cannot use ::{..} syntax because of
    // https://github.com/rust-lang/rust/issues/57411
    pub use crate::read::ReadHandle;
    pub use crate::read::ReadHandleFactory;
}

/// Helper types that give access to values inside the read half of an `evmap`.
pub mod refs {
    // Same here, ::{..} won't work.
    pub use super::values::Values;
    pub use crate::read::MapReadRef;
    pub use crate::read::ReadGuardIter;

    // Expose `ReadGuard` since it has useful methods the user will likely care about.
    #[doc(inline)]
    pub use left_right::ReadGuard;
}

// NOTE: It is _critical_ that this module is not public.
mod aliasing;

/// Options for how to initialize the map.
///
/// In particular, the options dictate the hashing function, meta type, and initial capacity of the
/// map.
pub struct Options<M, S>
where
    S: BuildHasher,
{
    meta: M,
    hasher: S,
    capacity: Option<usize>,
}

impl<M, S> fmt::Debug for Options<M, S>
where
    S: BuildHasher,
    M: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("meta", &self.meta)
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl Default for Options<(), RandomState> {
    fn default() -> Self {
        Options {
            meta: (),
            hasher: RandomState::default(),
            capacity: None,
        }
    }
}

impl<M, S> Options<M, S>
where
    S: BuildHasher,
{
    /// Set the initial meta value for the map.
    pub fn with_meta<M2>(self, meta: M2) -> Options<M2, S> {
        Options {
            meta,
            hasher: self.hasher,
            capacity: self.capacity,
        }
    }

    /// Set the hasher used for the map.
    ///
    /// # Safety
    ///
    /// This method is safe to call as long as the given hasher is deterministic. That is, it must
    /// yield the same hash if given the same sequence of inputs.
    pub unsafe fn with_hasher<S2>(self, hash_builder: S2) -> Options<M, S2>
    where
        S2: BuildHasher,
    {
        Options {
            meta: self.meta,
            hasher: hash_builder,
            capacity: self.capacity,
        }
    }

    /// Set the initial capacity for the map.
    pub fn with_capacity(self, capacity: usize) -> Options<M, S> {
        Options {
            meta: self.meta,
            hasher: self.hasher,
            capacity: Some(capacity),
        }
    }

    /// Create the map, and construct the read and write handles used to access it.
    ///
    /// If you want to use arbitrary types for the keys and values, use [`assert_stable`].
    #[allow(clippy::type_complexity)]
    pub fn construct<K, V>(self) -> (WriteHandle<K, V, M, S>, ReadHandle<K, V, M, S>)
    where
        K: sealed::StableHashEq + Eq + Hash + Clone,
        S: BuildHasher + Clone,
        V: sealed::StableHashEq + Eq + Hash,
        M: 'static + Clone,
    {
        unsafe { self.assert_stable() }
    }

    /// Create the map, and construct the read and write handles used to access it.
    ///
    /// # Safety
    ///
    /// This method is safe to call as long as the implementation of `Hash` and `Eq` for both `K`
    /// and `V` are deterministic. That is, they must always yield the same result if given the
    /// same inputs.
    #[allow(clippy::type_complexity)]
    pub unsafe fn assert_stable<K, V>(self) -> (WriteHandle<K, V, M, S>, ReadHandle<K, V, M, S>)
    where
        K: Eq + Hash + Clone,
        S: BuildHasher + Clone,
        V: Eq + Hash,
        M: 'static + Clone,
    {
        let inner = if let Some(cap) = self.capacity {
            Inner::with_capacity_and_hasher(self.meta, cap, self.hasher)
        } else {
            Inner::with_hasher(self.meta, self.hasher)
        };

        let (mut w, r) = left_right::new_from_empty(inner);
        w.append(write::Operation::MarkReady);

        (WriteHandle::new(w), ReadHandle::new(r))
    }
}

/// Create an empty eventually consistent map.
///
/// Use the [`Options`](./struct.Options.html) builder for more control over initialization.
///
/// If you want to use arbitrary types for the keys and values, use [`new_assert_stable`].
#[allow(clippy::type_complexity)]
pub fn new<K, V>() -> (
    WriteHandle<K, V, (), RandomState>,
    ReadHandle<K, V, (), RandomState>,
)
where
    K: sealed::StableHashEq + Eq + Hash + Clone,
    V: sealed::StableHashEq + Eq + Hash,
{
    Options::default().construct()
}

/// Create an empty eventually consistent map.
///
/// Use the [`Options`](./struct.Options.html) builder for more control over initialization.
///
/// # Safety
///
/// This method is safe to call as long as the implementation of `Hash` and `Eq` for both `K` and
/// `V` are deterministic. That is, they must always yield the same result if given the same
/// inputs.
#[allow(clippy::type_complexity)]
pub unsafe fn new_assert_stable<K, V>() -> (
    WriteHandle<K, V, (), RandomState>,
    ReadHandle<K, V, (), RandomState>,
)
where
    K: Eq + Hash + Clone,
    V: Eq + Hash,
{
    Options::default().assert_stable()
}

/// Create an empty eventually consistent map with meta information and custom hasher.
///
/// Use the [`Options`](./struct.Options.html) builder for more control over initialization.
///
/// # Safety
///
/// This method is safe to call as long as the implementation of `Hash` and `Eq` for both `K` and
/// `V`, and the implementation of `Hasher` for `S` are deterministic. That is, they must always
/// yield the same result if given the same inputs.
#[allow(clippy::type_complexity)]
pub unsafe fn with_hasher<K, V, M, S>(
    meta: M,
    hasher: S,
) -> (WriteHandle<K, V, M, S>, ReadHandle<K, V, M, S>)
where
    K: Eq + Hash + Clone,
    V: Eq + Hash,
    M: 'static + Clone,
    S: BuildHasher + Clone,
{
    Options::default()
        .with_hasher(hasher)
        .with_meta(meta)
        .assert_stable()
}

mod sealed {
    #[allow(unreachable_pub)]
    pub unsafe trait StableHashEq: std::hash::Hash + Eq {}

    macro_rules! yes_its_stable {
        ($($t:ty),*) => {
            $(unsafe impl StableHashEq for $t {})*
        };
    }

    yes_its_stable!(u8, u16, u32, u64, u128, usize);
    yes_its_stable!(i8, i16, i32, i64, i128, isize);
    yes_its_stable!(bool, char, String);

    unsafe impl<'a, T: StableHashEq> StableHashEq for &'a [T] {}
    unsafe impl<'a, T: StableHashEq> StableHashEq for &'a T {}
    unsafe impl<'a> StableHashEq for &'a str {}

    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    unsafe impl<T: StableHashEq> StableHashEq for Vec<T> {}
    unsafe impl<T: StableHashEq> StableHashEq for VecDeque<T> {}
    unsafe impl<T: StableHashEq> StableHashEq for BTreeSet<T> {}
    unsafe impl<K, V> StableHashEq for BTreeMap<K, V>
    where
        K: StableHashEq,
        V: StableHashEq,
    {
    }

    unsafe impl StableHashEq for () {}
    unsafe impl<T1> StableHashEq for (T1,) where T1: StableHashEq {}
    unsafe impl<T1, T2> StableHashEq for (T1, T2)
    where
        T1: StableHashEq,
        T2: StableHashEq,
    {
    }
    unsafe impl<T1, T2, T3> StableHashEq for (T1, T2, T3)
    where
        T1: StableHashEq,
        T2: StableHashEq,
        T3: StableHashEq,
    {
    }
    unsafe impl<T1, T2, T3, T4> StableHashEq for (T1, T2, T3, T4)
    where
        T1: StableHashEq,
        T2: StableHashEq,
        T3: StableHashEq,
        T4: StableHashEq,
    {
    }

    macro_rules! arr {
        ($($n:literal),*) => {
            $(unsafe impl<T: StableHashEq> StableHashEq for [T; $n] {})*
        }
    }

    arr!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16);
}
