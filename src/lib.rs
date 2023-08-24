// This is code for a memory scrubber.
//
// INTRODUCTION
// ============
// What is a memory scrubber and why would you use one?
//
// A memory scrubber is simply a piece of hardware or software that reads all
// bytes from a section of memory, usually from all of memory. This is an
// implementation of a software memory scrubber. When a processor reads from
// memory protected by an error correction code (ECC), it checks to see if
// there are errors in the piece of memory it has read. If so, in hardware or
// software, the ECC is used to correct the errors and the corrected value
// used to replace the bad value. The memory scrubber is run frequently
// enough that errors don't have a chance to accumulate
//
// This memory scrubber is specifically designed to allow the reduction of
// the impact of memory scrubbing. A simple memory scrubber might sequentially
// touch data one cache line apart. After touching a product of the number
// of cache lines in the cache and the number of ways per cache line, the
// previous contents of the cache will be complete evicted, requiring reloading
// when returning to the previous task. With 1024 cache lines and 4 ways,
// complete eviction will occur after 4096 touches.
//
// This memory scrubber is cache aware. As such, it scans through all addresses
// for a single cache line before advancing to the next cache line. With
// a 1 GB memory, 1024 cache lines, and a 64-byte cache line, it takes
// 16384 touches to evict a single cache line and 1677216 touches to evict
// the entire cache. This is a rate 2.4% of the simple approach.
//
// Using the cache aware memory scrubber is only useful if only part of
// memory is scrubbed at a time, but the rate of accumulation of error is
// slow enough that this is a very reasonable thing to do.
//
// NOTE: This is not intended to cover all possible cache
// implementations. Code to cover other variations is welcome.
//
// QUICK START
// ===========
// 1.   Add the lines:
//          use libmemscrub_arch::{BaseCacheDesc, CACHE_DESC, CacheDesc,
//              Cacheline, MemoryScrubber, ScrubArea};
//
// 2.   Determine values for and define the following:
//
//      a.  The integer type that your ECC unit operates on.
//
//          type MyECCData = u64;
//
//      b.  The number of MyECC items that fit in the longest cache line of any
//          cache level in uour sysem.
//
//              const MY_CACHELINE_ITEMS: usize = 8;
//
//      c.  The number of bits in the address used for the cache index for the
//          longest cache line in your sysem.
//
//              const MY_CACHE_INDEX_WIDTH: usize = 10;
//
//      d.  A BaseCacheline structure that uses the above to define what a cache
//          line looks like. This has a very specific memory layout, so it
//          must be specified as a C structure:
//
//              #[repr(C)]
//              struct MyBaseCacheline {
//                  data:   [MyECCData; MY_CACHELINE_ITEMS],
//              }
//
//              impl BaseCacheline for MyBaseCacheline {}
//
//
// 2.   Define and implement a BaseCacheDesc structure for your cache line. This
//      requires implementing the function cache_index_width(), which returns
//      the cache line width determined above, and read_cacheline(), which
//      causes the entire cacheline to be read:
//
//          struct MyBaseCacheDesc {
//              cache_index_width: usize,
//          }
//
//          impl BaseCacheDesc for MyBaseCacheDesc {
//              fn cache_index_width(&self) -> usize { self.cache_index_width }
//              fn read_cacheline(&mut self,
//                  cacheline_ptr: *const MyBaseCacheline) {
//                  let cacheline = unsafe { &*cacheline_ptr };
//                  let cacheline_data = &cacheline.data[0];
//                  let _dummy = unsafe { ptr::read(cacheline_data) };
//              }
//          }
//
//          static MY_CACHE_DESC: MyBaseCacheDesc = MyBaseCacheDesc {
//              cache_index_width:  MY_CACHE_INDEX_WIDTH,
//          };
// 
// 3.   Create an array with the virtual addresses of all memory areas to
//      scrub:
//
//          let my_scrub_addrs = [
//              ScrubArea {
//                  start: 0xa0000000 as *const u8,
//                  end: 0xbfffffff as *const u8,
//              },
//              ScrubArea {
//                  start: 0xd0000000 as *const u8,
//                  end: 0xefffffff as *const u8,
//              },
//          ];
//
// 4.   The simplest thing to do is to use the autoscrub() function. The work
//      it does can be broken down, see below. Using autoscrub() consists of:
//      a.  Create a structure implementing the AutoScrubDesc trait. The
//          only thing that needs to be defined is a function that returns
//          the number of bytes to be scrubbed. If it returns zero, the
//          autoscrub() function will return:
//
//              struct MyAutoScrubDesc {
//                  scrub_size: usize,
//              }
//
//              impl AutoScrubDesc for MyAutoScrubDesc {
//                  fn next(&mut self) -> usize {
//                      self.scrub_size
//                  }
//              }
//
//              let mut autoscrub_desc = TestAutoScrubDesc {
//                  scrub_size: my_cache_desc.cacheline_size() * 5000,
//              };
//
//      c.  Invoke autoscrub():
//
//              let scrub = AutoScrub.autoscrub(cache_desc, &scrub_areas,
//                  &mut autoscrub_desc)?;
//
// DETAILS
// =======
// ECCs are limited in the number of errors they can correct. These errors
// generally accumulate over time. So long as memory is read often enough,
// correction is applied with enough frequency that the number of errors
// stays within the bounds of what is correctable. However, a piece of
// memory that is rarely accessed can accumulate multiple errors. When that
// memory is eventually used, it will not be possible to determine the corrected
// value and a fatal error will result. This is where a memory scrubber comes
// in.
//
// In general, memory should be scrubbed at a rate high enough that the number
// of accumulated errors remains low enough that the probability that there are
// memory words with uncorrectable errors is extremely low. Since it isn't
// possible to predict which areas of memory are read frequently enough to
// avoid error accumulation, the usual practice is to scan all of memory. With
// modern systems, this can be quite be a large amount of work and the
// scrubbing work is broken into smaller pieces to avoid any significant
// amount of performance impact.
//
// The choice of how often memory is scrubbed depends on:
// o    The probability that an uncorrectable number of errors will accumulate
//      in a particular section
// o    How many sections of memory are present in the system
// o    The goal for the probability that a fault due to an uncorrectable
//      number of errors anywhere in memory will occur.
//
// Choosing how the scrubbing work is divided into smaller piece depends on
// many implementation details, like:
// o    Will the scrubbing be done with preemption blocked?
// o    How long does it take to scrub each section of memory?
// o    What is the overhead of entering and leaving the scrubbing code each
//      time it is run?
//
// One key performance impact of memory scrubbing is that read memory will
// evict the memory cache contents being used by other software on the system,
// with modified cache lines being written to memory.  When that software
// resumes, it will have to re-read all the data it wants to use. This may
// cause a substantial performance impact all at once.  This library is written
// to perform its reads all of memory corresponding to a single cache line at
// a time. If memory scrubbing is broken into smaller chunks, data will be
// evicted from only a few cache lines each time scrubbing is done, producing
// a more even performance impact.
//
// CACHE ORGANIZATION AND ADDRESSES
// ================================
//
// This code assumes that an address can be broken into three parts:
//
//       _____________________________________________________________
//      |                                |               |            |
//      |     Address upper bits         |  Cache index  | Cache line |
//      |                                |               |   index    |
//      |________________________________|_______________|____________|
//
// The cache index is the index of the cache line in the cache. The cache
// line index is the index of a particular byte in the cache line indicated
// by the cache index. The upper bits of the address might be used to select
// a specific way within the cache line specified by the cache index but don't
// usually otherwise participate in cache operations.
//
// USAGE
// =====
// FIXME; This all has to be reviewed.
// To use this, it recommended you do the following:
//
// 1.   Determine a suitable data type to represent the size of object that
//      is used by the unit the computes the ECC. This is probably either u32
//      or u64, and we'll call it ECCData here, though you can use anything
//      appropriate to your error correction hardware.
//
// 2.   Define the structure of a cache line by implementing BaseCacheline for
//      the particular layout for your processor. We'll call the structure
//      MyBaseCacheline. It usually the case that cache lines are arrays of ECCData
//      items, such as:
//
//          #[repr(C)]
//          struct MyBaseCacheline {
//              data: [ECCData; 8];
//          }
//
//      Since many systems have more than one level of cache memory, note
//      that this should be the longest cache line in in the system.
//
// 3.   Determine the address of the first and last bytes of the memory area
//      which you want to scrub. Call these my_start and my_end. The start
//      must be a multiple of the cache line size, the end must be one less
//      than a multiple of the cache line size. If your cache has multiple ways
//      (likely), the cache line size is the number of bytes in a single way.
//
// 3.   Create a structure that holds definitions of your cache. This is
//      an implementation of the BaseCacheDesc trait. For example purposes, call
//      this MyBaseCacheDesc. In most cases, the default functions provide
//      everything you need, so only things you need to define are the
//      following:
//
//      a.  The cache_index_width() function, which returns the number of
//          bits in the cache index portion of an address. For example, a
//          ten-bit wide cache index would be implemented by:
//
//              fn cache_index_width(&self) -> usize {
//                  10
//              }
//
//      b.  A function that will cause all bytes in a cache line to be read
//          and checked for a correct ECC.  If the entire cache is read when
//          any element is read, this can be done with a minimal amount of
//          unsafe code:
//
//              fn read_cacheline(&mut self, cacheline_ptr: *const MyBaseCacheline) {
//                  // Get a safe reference to the cache line
//                  let cacheline = unsafe {
//                      &*cacheline_ptr
//                  };
//                  // Get a reference to the first element
//                  let cacheline_data = &cacheline.data[0];
//                  // Read from the first element
//                  let _dummy = unsafe {
//                      ptr::read(cacheline_data)
//                  };
//              }
//
//          There is a conceivable architecture in which only part of the
//          longest cache line will be read when a single element is read.
//          Since any memory not read will not be checked for errors, it is
//          important that this function implement a full cache line read.
//          Check your processor's reference manual to determine how to do
//          this.
//
// 4.   Create a new MemoryScrubber:
//
//          let scrubber = match MemoryScrubber::<MyBaseCacheline>::
//              new(&MyBaseCacheDesc::<MyBaseCacheline> {...}, my_start, my_end) {
//              Err(e) => ...
//
// 5.   Scrub some number of bytes. You could scrub a quarter of the memory area
//      with:
//
//          match scrubber.scrub(size / 4) {
//              Err(e) => ...
//
//      The size passed to scrub_scrub_areIa() must be a multiple of the cache
//      line size.
//
// BREAKING UP SCANS
// =================
// The decision of how to break up a single scan of all of memory depends on
// system factors such as:
// o    Is the scan preemptible?
// o    Does the scan cause a context switch and how many can be tolerated
//      in a given interval?
// o    What is the performance impact of evicting and reloading the section
//      of cache corresponding to the scrubbed memory.
//
// FREQUENCY OF SCANS
// ==================
// Start by determining the number of errors an ECC unit that operates on
// words with w bits can correct.
//
// GPT4 Query
// ----------
//  Assume a memory with S words of W bits, with the probability that a single
//  bit will be flipped in time Tf is P. What is probability that at least one
//  word will have more than N bits flipped in the interval T?
//
// Unedited answer (has not yet been verified)
// -------------------------------------------
//  The problem you're describing involves complex probabilities and involves
//  calculations related to binomial distributions. While there isn't a single
//  "mathematical function" that directly provides the answer, you can break
//  down the problem into components based on probability theory. Here's a
//  breakdown of the approach in mathematical terms:
//
//  Let:
//
//  - `S` be the number of words.
//  - `W` be the number of bits in a word.
//  - `P` be the probability that a single bit will be flipped in time Tf.
//  - `N` be the maximum number of flipped bits in a word.
//  - `k` be the number of bits flipped in a word (0 <= k <= W).
//
//  The probability that exactly `k` bits are flipped in a single word can be
//  calculated using the binomial distribution formula:
//
//  ```
//  P(k) = C(W, k) * (1 - P)^(W - k) * P^k
//  ```
//
//  Where `C(W, k)` is the binomial coefficient, given by:
//
//  ```
//  C(W, k) = W! / (k! * (W - k)!)
//  ```
//
//  The probability that a single word doesn't have more than `N` bits flipped
//  is the sum of probabilities for `k` from 0 to `N`:
//
//  ```
//  P_single_word = Σ(P(k)) for k = 0 to N
//  ```
//
//  Finally, the probability that at least one word has more than `N` bits
//  flipped in the interval T can be calculated using the complement rule:
//
//  ```
//  P_at_least_one_word = 1 - (1 - P_single_word)^S
//  ```
//
//  Keep in mind that these formulas involve factorials and exponentials, which
//  can lead to large computations for larger values of `S`, `W`, and `N`. You
//  might need to use specialized libraries or numerical approximations if you
//  intend to calculate these probabilities for significant values of these
//  parameters.
//
// ChatGPT Result For Determination Of The Time Until The First Word Goes Bad
// --------------------------------------------------------------------------
//  To compute the time interval for a given `P_at_least_one_word`, you would
//  need to rearrange the formula for `P_at_least_one_word` to solve for the
//  time interval `T`:
//
//  ```
//  P_at_least_one_word = 1 - (1 - P_single_word)^S
//  ```
//
//  Let's rearrange the formula:
//
//  ```
//  1 - P_at_least_one_word = (1 - P_single_word)^S
//  ```
//
//  ```
//  T = (-ln(1 - P_at_least_one_word)) / ln(1 - P_single_word)
//  ```
//
//  In this formula, `ln` represents the natural logarithm function. You can
//  use Rust's `f64` math functions to compute this.
//
//  Here's how you can implement it in Rust:
//
//  ```rust
//  fn compute_time_interval(P_at_least_one_word: f64, P_single_word: f64, S: u32) -> f64 {
//      let numerator = -(1.0 - P_at_least_one_word).ln();
//      let denominator = (1.0 - P_single_word).ln();
//
//      numerator / (denominator * f64::from(S))
//  }
//
//  fn main() {
//      let P_at_least_one_word = 0.9; // Desired probability
//      let P_single_word = 0.01; // Probability for a single word
//      let S = 10; // Number of words
//
//      let time_interval = compute_time_interval(P_at_least_one_word, P_single_word, S);
//      println!("Time Interval: {}", time_interval);
//  }
//  ```
//
//  Replace the values of `P_at_least_one_word`, `P_single_word`, and `S` with
//  your specific values. Keep in mind that these calculations might not be
//  feasible for very small values of `P_at_least_one_word` or `P_single_word`
//  due to the precision of floating-point arithmetic. Additionally, the `ln`
//  function might return NaN (Not-a-Number) for certain inputs, so you should
//  handle potential edge cases in your code.
//
// So, that determines how often the entire memory must be scanned. Note that
// above, S is the number of words of memory. The total number of 8-bit bytes in
// memory is S * (W / 8).
//
// NOTE: The above assumes that, once a bit is flipped, it stays flipped. The
// probability of a bit being inverted, then inverted again is small enough
// that it can be ignored.

use std::cell::RefCell;
use std::iter;
use std::marker::PhantomData;
use std::rc::Rc;
use thiserror::Error;

// C-language interface
//

// Structure used to define an area to be scribbed
// start - lowest address of the area. Must be a multiple of the cache line size
// end - address of the last byte of the area. Must be one less than a multiple
//      of the cache line size
#[derive(Clone, Debug)]
#[repr(C)]
pub struct ScrubArea {
    pub start:              *const u8,
    pub end:                *const u8,
}

// End of C code, it's all Rust from here on in.

// Data type that can hold any address for manipulation as an integer
type Addr = usize;

pub trait BaseCacheline {
}

// Describe cache parameters and pull in all elements of the cache line.
pub trait BaseCacheDesc<T: BaseCacheline> {
    // NOTE: You are unlikely to ever need to implement this
    // Return the number of bits required to hold an index into the bytes of
    // a cache line. So, if you have an eight-byte cache line (unlikely), this
    // would return 3.
    fn cacheline_width(&self) -> usize {
        usize::BITS as usize - 1 - std::mem::size_of::<T>()
            .leading_zeros() as usize
    }

    // NOTE: You are unlikely to ever need to implement this
    // Return the number of bytes in the cache line. A cache line width of 4
    // bits will have a cache line size of 16 bytes.
    fn cacheline_size(&self) -> usize {
        1 << self.cacheline_width()
    }

    // Return the number of bits used to index into the cache, i.e. the index
    // of a cache line in the cache. A cache with 1024 lines will have an
    // index using 10 bits.
    fn cache_index_width(&self) -> usize;

    // NOTE: You are unlikely to ever need to implement this
    // Return the number of cache lines in the index. For a 1024 line cache
    // and a 16 byte cache line, this will be 64.
    fn cache_lines(&self) -> usize {
        1 << self.cache_index_width()
    }

    // This function is given a pointer to a cache line-aligned address with
    // as many bytes as are in a cache line. The implementation should do
    // whatever is necessary to ensure all bytes are read in order to trigger
    // a fault if any bits have an unexpected value. So long as the number
    // of bad bits is small enough (ECC-dependent), corrected data should
    // be written back to that location, preventing the accumulation of so many
    // bad bits that the correct value cannot be determined.
    fn read_cacheline(&mut self, cacheline_ptr: *const T);

    // Return the size of a ScrubArea in cachelines
    fn size_in_cachelines(&self, scrub_area: &ScrubArea) -> usize {
        let start_in_cachelines =
            scrub_area.start as usize >> self.cacheline_width();
        // This will truncate the number of cache lines by one
        let end_in_cachelines =
            scrub_area.end as usize >> self.cacheline_width();
        (end_in_cachelines - start_in_cachelines) + 1
    }

    // Returns the cache index part of the address
    fn cache_index(&self, p: *const u8) -> usize {
        let width = self.cacheline_width();
        let mask = (1 << self.cache_index_width()) - 1;
        (p as Addr >> width) & mask
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq)]
#[repr(C)]
pub enum Error {
    #[error("Start address must be aligned on cache line boundary")]
    UnalignedStart,

    #[error("End address must be one less than a cache line boundary")]
    UnalignedEnd,

    #[error("Number of bytes must be a multiple of the cache size")]
    UnalignedSize,

    #[error("No ScrubAreas supplied")]
    NoScrubAreas,

    #[error("ScrubArea is empty")]
    EmptyScrubArea,

    #[error("Internal Error: Iterator failed")]
    IteratorFailed,
}

pub trait BaseAutoScrubDesc {
    fn next(&mut self) -> usize;
}

pub struct BaseAutoScrub<'a, T:BaseCacheDesc<U>, U:BaseCacheline> {
    scrubber:   MemoryScrubber<'a, T, U>,
    desc:       &'a mut dyn BaseAutoScrubDesc,
}

impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> BaseAutoScrub<'a, T, U> {
    pub fn autoscrub(cache_desc: &'a mut T, scrub_areas: &'a [ScrubArea],
            desc: &'a mut dyn BaseAutoScrubDesc) ->
        Result<usize, Error> {
        let scrubber = MemoryScrubber::new(cache_desc, scrub_areas)?;

        let mut autoscrub = BaseAutoScrub {
            scrubber: scrubber,
            desc: desc,
        };

        loop {
            let n = autoscrub.desc.next();
            if n == 0 {
                return Ok(n);
            }
            autoscrub.scrubber.scrub(n)?;
        }
    }
}

// Memory scrubber
// cache_desc - Description of the cache
// scrub_areas - ScrubAreas being scrubbed
// iterator - MemoryScrubberIterator used to walk through the memory being
//      scrubbed
pub struct MemoryScrubber<'a, T: BaseCacheDesc<U>, U: BaseCacheline> {
    cache_desc:     Rc<RefCell<&'a mut T>>, //<'a, BaseCacheline>,
    scrub_areas:    &'a [ScrubArea],
    iterator:       Option<MemoryScrubberIterator<'a, T, U>>,
}

impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> MemoryScrubber<'a, T, U> {

    // Create a new memory scrubber
    // cache_desc - Description of the cache
    // start - Virtual address of memory being scrubbed
    // end - Virtual address of the last byte of memory to be scrubbed
    pub fn new(cache_desc: &'a mut T, scrub_areas: &'a [ScrubArea]) ->
        Result<MemoryScrubber<'a, T, U>, Error> {

        if scrub_areas.len() == 0 {
            return Err(Error::NoScrubAreas);
        }

        let cacheline_size = {
            cache_desc.cacheline_size()
        };

        // Look for all possible errors in all ScrubAreas.
        for scrub_area in scrub_areas {
            // The code will actually handle this just fine, but it's extra
            // effort to no benefit, so it is expected to be a user error.
            if scrub_area.start == scrub_area.end {
                return Err(Error::EmptyScrubArea);
            }

            let start_addr = scrub_area.start as Addr;
            let end_addr = scrub_area.end as Addr;

            if (start_addr & (cacheline_size - 1)) != 0 {
                return Err(Error::UnalignedStart);
            }

            if (end_addr & (cacheline_size - 1)) != cacheline_size - 1 {
                return Err(Error::UnalignedEnd);
            }
        }

        let cache_desc_rc = Rc::new(RefCell::new(cache_desc));

        Ok(MemoryScrubber::<'a, T, U> {
            cache_desc:     cache_desc_rc,
            scrub_areas:    scrub_areas,
            iterator:       None,
        })
    }

    // Scrub some number of bytes. This could be larger than the total memory
    // area, in which case the scrubbing will start again at the beginning
    // of the memory area, but it seems unlikely that this would be useful.
    // n - Number of bytes to scrub
    pub fn scrub(&mut self, n: usize) -> Result<(), Error> {
        let cacheline_width = {
            self.cache_desc.borrow().cacheline_width()
        };

        let cacheline_size = {
            self.cache_desc.borrow().cacheline_size()
        };

        if (n & (cacheline_size - 1)) != 0 {
println!("n {}", n);
            return Err(Error::UnalignedSize);
        }

        // Convert to the number of cachelines to scrub
        let cachelines_to_scrub = n >> cacheline_width;

        for _i in 0..cachelines_to_scrub {
            // Get the next area to scrub. If we don't have an iterator, get
            // one
            let p: *mut U;

            loop {
                if self.iterator.is_none() {
                    let cache_desc = self.cache_desc.clone()
                        as Rc<RefCell<&mut T>>;
                    self.iterator =
                        Some(MemoryScrubberIterator::<T, U>::new(cache_desc,
                        &self.scrub_areas));
                }

                let next = self.iterator.as_mut().unwrap().next();

                match next {
                    None => self.iterator = None,
                    Some(this_p) => {
                        p = this_p;
                        break;
                    },
                }
            }

            let cd = &mut self.cache_desc.borrow_mut();
            cd.read_cacheline(p);
        }

        
        Ok(())
    }
}

pub struct MemoryScrubberIterator<'a, T, U> {
    cache_desc:     Rc<RefCell<&'a mut T>>,
    scrub_areas:    &'a [ScrubArea],
    iterator:       Option<ScrubAreaIterator<'a, T, U>>,
    index:          usize,
}

impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> MemoryScrubberIterator<'a, T, U> {
    pub fn new(cache_desc: Rc<RefCell<&'a mut T>>,
        scrub_areas: &'a [ScrubArea]) ->
        MemoryScrubberIterator<'a, T, U> {

        MemoryScrubberIterator {
            cache_desc:     cache_desc,
            scrub_areas:    scrub_areas,
            iterator:       None,
            index:          0,
        }
    }
}

impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> iter::Iterator for
    MemoryScrubberIterator<'_, T, U> {
    type Item = *mut U;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.index == self.scrub_areas.len() {
                return None;
            }

            if self.iterator.is_none() {
                self.iterator =
                    Some(ScrubAreaIterator::<T, U>::new(self.cache_desc.clone(),
                    &self.scrub_areas[self.index]));
            }

            match self.iterator.as_mut().unwrap().next() {
                None => self.iterator = None,
                Some(p) => return Some(p),
            }

            self.index += 1;
        }
    }
}

// ScrubAreaIterator to scan a ScrubArea, keeping on a single cache line as
// long as possible.
//
// scrub_area:  Specifies the address of the scrub area
// index:       Value that, when added to the cache index value of start, yields
//              the index of the cache line being scrubbed
// offset:      Number of cache lines between the first address corresponding to
//              the given cache index and the address that will be read. This is
//              a multiple of the number cache lines in the cache.
// _marker:     Forces U to be recognized as used because something in the
//              compiler doesn't realize this. FIXME: remove _marker
pub struct ScrubAreaIterator<'a, T, U> {
    cache_desc: Rc<RefCell<&'a mut T>>,
    scrub_area: ScrubArea,
    index:      usize,
    offset:     usize,
    _marker:    PhantomData<U>
}

impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> ScrubAreaIterator<'a, T, U> {
    // Create a new ScrubAreaIterator.
    // scrub_area: Memory over which we Iterate
    //
    // Returns: Ok(ScrubAreaIterator) on success, Err(Error) on failure
    pub fn new(cache_desc: Rc<RefCell<&'a mut T>>,
        scrub_area: &'a ScrubArea) -> ScrubAreaIterator<'a, T, U> {

        ScrubAreaIterator {
            cache_desc: cache_desc,
            scrub_area: scrub_area.clone(),
            index:      0,
            offset:     0,
            _marker:    PhantomData,
        }
    }
}

// Return a pointer into a series of BaseCacheline items. To get a byte address
// from the return value of next(), call it ret_val, use:
impl<'a, T: BaseCacheDesc<U>, U: BaseCacheline> iter::Iterator for ScrubAreaIterator<'a, T, U> {
    type Item = *mut U;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If we've scanned all cache lines, we're finished.
            if self.index == self.cache_desc.borrow().cache_lines() {
                return None;
            }

            // We need to get the offset, in cache lines, of the address that
            // we are scrubbing. First we sum:
            //
            // o    The offset into the cache of the starting address
            // o    The offset into the cache of the set of cache lines we
            //      are scrubbing
            //
            // This, modulo the cache size, is the cache index for the addresses
            // in a pass through that cache index.
            let cd = &self.cache_desc.borrow() as &T;
            let offset = self.index + self.offset;
            let size = cd.size_in_cachelines(&self.scrub_area);

            if offset < size {
                let start = self.scrub_area.start as *const U;
                let res = unsafe {
                    start.offset(offset as isize)
                };
                self.offset += self.cache_desc.borrow().cache_lines();
                return Some(res as *mut U);
            }
            self.index += 1;
            self.offset = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{RefCell};
    use std::ptr;
    use std::rc::Rc;
    use std::time::Instant;

    use crate::{Addr, AutoScrub, AutoScrubDesc, BaseCacheDesc, BaseCacheline, Error,
        MemoryScrubber, ScrubArea};

    // Cache characteristics
    // BASIC_CACHELINE_WIDTH - number of bits required to index a byte in a
    //      cache line
    // BASIC_CACHE_INDEX_WIDTH - number of bits used as a cache line index in
    //      the cache
    // BASIC_MEM_SIZE - Cache size, in bytes
    const BASIC_ECCDATA_WIDTH: usize = usize::BITS as usize - 1 -
        std::mem::size_of::<BasicECCData>() .leading_zeros() as usize;
    const BASIC_CACHELINE_WIDTH: usize = 6 + BASIC_ECCDATA_WIDTH;
    const BASIC_CACHE_INDEX_WIDTH: usize = 10;
    const BASIC_MEM_SIZE: usize =
        1 << (BASIC_CACHELINE_WIDTH + BASIC_CACHE_INDEX_WIDTH);

    // ECCData - The data size used to compute the ECC for basic tests
    // BasicCacheline - the data type of a cache line
    type BasicECCData = u64;

    #[repr(C)]
    struct BasicCacheline {
        data:   [BasicECCData;
            (1 << BASIC_CACHELINE_WIDTH) / std::mem::size_of::<BasicECCData>()],
    }

    impl BaseCacheline for BasicCacheline {}

    // BasicCacheDesc - Description of the cache for basic tests
    // cache_index_width - Number of bits of the cache index.
    #[derive(Clone, Copy, Debug)]
    struct BasicCacheDesc {
        cache_index_width:        usize,
    }

    impl BaseCacheDesc<BasicCacheline> for BasicCacheDesc {
        fn cache_index_width(&self) -> usize {
            self.cache_index_width
        }

        fn read_cacheline(&mut self, cacheline_ptr: *const BasicCacheline) {
            let cacheline = unsafe {
                &*cacheline_ptr
            };
            let cacheline_data = &cacheline.data[0];
            let _dummy = unsafe {
                ptr::read(cacheline_data)
            };
        }
    }

    // Cache descriptor to pass to the memory scrubbing functions.
    static BASIC_CACHE_DESC: BasicCacheDesc = BasicCacheDesc {
        cache_index_width:        BASIC_CACHE_INDEX_WIDTH,
    };

    // Cache characteristics
    // TOUCHING_CACHELINE_WIDTH - number of bits required to index a byte in a
    //  cache line
    // TOUCHING_CACHE_INDEX_WIDTH - number of bits used as a cache line index
    //  in the cache
    // TOUCHING_CACHE_LINES - number of cache lines
    const TOUCHING_ECCDATA_WIDTH: usize = usize::BITS as usize - 1 -
        std::mem::size_of::<TouchingECCData>() .leading_zeros() as usize;
    const TOUCHING_CACHELINE_WIDTH: usize = 3 + TOUCHING_ECCDATA_WIDTH;
    const TOUCHING_CACHE_INDEX_WIDTH: usize = 10;
    const TOUCHING_CACHE_LINES: usize = 1 << TOUCHING_CACHE_INDEX_WIDTH;

    // GUARD_LINES - Number of cache line size items we allocate but don't
    //      touch, to verify we don't touch the wrong place.
    // TOUCHING_CACHE_NUM_TOUCHED - Number of cache footprints we use for
    //      testing
    // TOUCHING_SANDBOX_SIZE - Number of cachelines we actually expect to
    //      touch
    const GUARD_LINES: usize = TOUCHING_CACHE_LINES;
    const TOUCHING_CACHE_NUM_TOUCHED: usize = 3;
    const TOUCHING_SANDBOX_SIZE: usize =
        TOUCHING_CACHE_LINES * TOUCHING_CACHE_NUM_TOUCHED;

    // TouchingECCData - The data size used to compute the ECC for basic tests
    type TouchingECCData = u64;
    const TOUCHING_ECC_DATA_SIZE: usize =
        std::mem::size_of::<TouchingECCData>();

    // TouchingCacheline - the data type of a cache line
    #[repr(C)]
    struct TouchingCacheline {
        data:   [TouchingECCData;
            (1 << TOUCHING_CACHELINE_WIDTH) / TOUCHING_ECC_DATA_SIZE],
    }

    impl BaseCacheline for TouchingCacheline {}

    // Description of memory that is read into by the read_cacheline() function.
    // This keeps the actually allocation together with the pointer into that
    // allocation so that things go out of scope at the same time.
    //
    // mem_area - Vec<u8> of elements that can be read by read_cacheline()
    // start - Cache size-aligned pointer of the first byte to use in mem_area
    // end - Pointer to the last byte
    #[derive(Clone, Debug)]
    struct Mem {
        mem_area:   Vec<u8>,
        scrub_area: ScrubArea,
    }

    impl Mem {
        // Allocates a memory area on a cache line boundary
        //
        // cacheline_size - Number of bytes in a cache line
        // size - The number of bytes to allocate
        // 
        // Returns: a Mem with a Vec<u8>. The size of the Vec<u8> is
        // opaque but the p element in the Mem has at least size bytes
        // starting at a cache line aligned section of memory. The size element
        // is the size used to call this function.
        fn new<T>(size: usize) ->
            Result<Mem, Error> {
            let cacheline_size = std::mem::size_of::<T>();

            if (size & (cacheline_size - 1)) != 0 {
                return Err(Error::UnalignedSize);
            }

            // Allocate memory, which includes a cache size-sided area before
            // what we are touching. These areas should not be touched by
            // scrubbing.
            let mem_area: Vec<u8> = vec![0; cacheline_size + size];

            // Now find the first cache line aligned pointer
            let start_addr = (mem_area.as_ptr() as Addr + cacheline_size - 1) &
                !(cacheline_size - 1);
            let start = start_addr as *const u8;
            let end = (start_addr + size - 1) as *const u8;

            Ok(Mem {
                mem_area:   mem_area,
                scrub_area: ScrubArea { start: start, end: end },
            })
        }
    }

    // This clues the compiler in that I know what I'm doing by having a
    // *const pointer in the struct
    unsafe impl Sync for Mem {}

    // Data structure for keep track of how many times a given address has
    // been read
    // mem:     Boundaries of the address covered by this structure
    // n_reads: Counters
    #[derive(Clone, Debug)]
    struct ReadInfo {
        mem:        Mem,
        n_reads:    Option<Vec<NRead>>
    }

    impl ReadInfo {
        // Allocate a vector for counters
        // size:    Size in cache lines
        // mem:     Associated memory
        fn new(size: usize, mem: Mem) -> ReadInfo {
            let n_reads = vec![0; size];
            ReadInfo {
                mem:        mem,
                n_reads:    Some(n_reads),
            }
        }
    }

    // Type used for the read counter
    type NRead = u8;

    // Cache descriptor to pass to the memory scrubbing functions.
    static TOUCHING_CACHE_DESC: TouchingCacheDesc =
        TouchingCacheDesc {
        cache_index_width:  TOUCHING_CACHE_INDEX_WIDTH,
        read_infos:         None,
    };

    // TouchingCacheDesc - Description of the cache for basic tests
    // cache_index_width - Number of times this cacheline was iit during the
    //      scrub
    // read_infos:          Array of ReadInfo items>
    #[derive(Clone, Debug)]
    struct TouchingCacheDesc {
        cache_index_width:  usize,
        read_infos:         Option<Vec<ReadInfo>>,
    }

    impl TouchingCacheDesc {
        // Set up a new TouchingCacheDesc
        // sizes: array of sizes of memory areas
        fn new<BasicCacheline>(sizes: &[usize]) -> TouchingCacheDesc {
            let mut touching_cache_desc = TOUCHING_CACHE_DESC.clone();
            let mut read_infos = vec!();

            for size in sizes {
                let mem =
                    match Mem::new::<TouchingCacheline>(*size) {
                    Err(e) => panic!("Memory allocation error: {}", e),
                    Ok(mem) => mem,
                };

                let n_reads_size = GUARD_LINES +
                    touching_cache_desc.size_in_cachelines(&mem.scrub_area) +
                    GUARD_LINES;
                read_infos.push(ReadInfo::new(n_reads_size, mem));
            }

            touching_cache_desc.read_infos = Some(read_infos);
            touching_cache_desc
        }

        // Compute the index into the n_read array for this address. This
        // array has GUARD_LINES elements surrounding the actual counts.
        // cacheline_ptr: Pointer to the address
        fn read_index(&mut self, cacheline_ptr: *const TouchingCacheline) ->
            usize {
            let cacheline_addr = cacheline_ptr as Addr;
            let cacheline_size = {
                self.cacheline_size ()
            };

            let read_info = self.find_read_info(cacheline_ptr);
            let n_read_size = std::mem::size_of::<NRead>();
            let n_n_reads = read_info.mem.mem_area.len() / n_read_size;
            let start_addr = read_info.mem.scrub_area.start as Addr;

            let offset = (cacheline_addr - start_addr) / cacheline_size;
            let index = GUARD_LINES + offset;
            assert!(index >= GUARD_LINES);
            assert!(index < n_n_reads + GUARD_LINES);
            index
        }

        // Returns a reference to n_reads[], the array of count read counts
        fn get_n_reads<'a>(&'a mut self,
            cacheline_ptr: *const TouchingCacheline) ->
            &'a mut Vec<NRead> {
            let read_info = self.find_read_info(cacheline_ptr);
            read_info.n_reads.as_mut().unwrap()
        }

        fn find_read_info<'a>(&'a mut self,
            cacheline_ptr: *const TouchingCacheline) ->
            &'a mut ReadInfo {
            let cacheline_addr = cacheline_ptr as Addr;
            let read_infos: &mut Vec<ReadInfo> =
                self.read_infos.as_mut().unwrap();
            
            for search_read_info in read_infos.iter_mut() {
                let scrub_area = &search_read_info.mem.scrub_area;
                let start_addr = scrub_area.start as Addr;
                let end_addr = scrub_area.end as Addr;

                if cacheline_addr >= start_addr &&
                    cacheline_addr <= end_addr {
                    return search_read_info;
                }
            }

            // If we failed, it's because the cache addess wasn't in any of
            // the ScrubAreas.
            panic!("Unable to find address {:x}", cacheline_addr);
        }
    }

    impl BaseCacheDesc<TouchingCacheline> for TouchingCacheDesc {
        fn cache_index_width(&self) -> usize {
            self.cache_index_width
        }

        fn read_cacheline(&mut self, cacheline_ptr: *const TouchingCacheline) {
            // Do the read
            let cacheline = unsafe {
                &*cacheline_ptr
            };
            let cacheline_data = &cacheline.data[0];
            let _dummy = unsafe {
                ptr::read(cacheline_data)
            };

            // Update the read count
            let index = {
                self.read_index(cacheline_ptr)
            };
            let n_reads = {
                self.get_n_reads(cacheline_ptr)
            };

            n_reads[index] += 1;
        }
    }

    // This clues the compiler in that I know what I'm doing by having a
    // *const pointer in the struct
    unsafe impl Sync for TouchingCacheDesc {}

    // Verify that an error is returned if the starting address is not
    // aligned on a cache line boundary
    #[test]
    fn test_unaligned_start() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();
        let mut mem =
            match Mem::new::<BasicCacheline>(BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.start = unsafe {
            mem.scrub_area.start.offset(1)
        };

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheDesc, BasicCacheline>::new(basic_cache_desc,
            &scrub_areas);
        assert!(memory_scrubber.is_err());
        assert_eq!(memory_scrubber.err().unwrap(),
            Error::UnalignedStart);
    }

    // Verify that an error is returned if the ending address is not
    // aligned on a cache line boundary
    #[test]
    fn test_unaligned_end() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();
        let mut mem =
            match Mem::new::<BasicCacheline>(BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.end = unsafe {
            mem.scrub_area.end.offset(-1)
        };

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheDesc, BasicCacheline>::new(basic_cache_desc,
            &scrub_areas);
        assert!(memory_scrubber.is_err());
        assert_eq!(memory_scrubber.err().unwrap(),
            Error::UnalignedEnd);
    }

    // Verify that an error is returned if the size is zero.
    #[test]
    fn test_null_areas() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();

        let scrub_areas = [];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheDesc, BasicCacheline>::new(basic_cache_desc,
            &scrub_areas);
        assert!(memory_scrubber.is_err());
        assert_eq!(memory_scrubber.err().unwrap(),
            Error::NoScrubAreas);
    }

    // Verify that an error is returned if the size is zero.
    #[test]
    fn test_zero_size() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();
        let mut mem =
            match Mem::new::<BasicCacheline>(BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.end = mem.scrub_area.start;

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheDesc, BasicCacheline>::new(basic_cache_desc,
            &scrub_areas);
        assert!(memory_scrubber.is_err());
        assert_eq!(memory_scrubber.err().unwrap(),
            Error::EmptyScrubArea);
    }

    // Verify that a small scrub with good parameters can be done.
    #[test]
    fn test_aligned() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();
        let cacheline_size = basic_cache_desc.cacheline_size();
        let mem =
            match Mem::new::<BasicCacheline>(basic_cache_desc.cacheline_size() *
                basic_cache_desc.cache_lines() * 14) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };

        let scrub_areas = [mem.scrub_area];
        let mut memory_scrubber =
            match MemoryScrubber::<BasicCacheDesc, BasicCacheline>::new(basic_cache_desc,
                &scrub_areas) {
            Err(e) => panic!("MemoryScrubber::new() failed {}", e),
            Ok(scrubber) => scrubber,
        };

        if let Err(e) = memory_scrubber.scrub(cacheline_size * 10) {
            panic!("scrub failed: {}", e);
        }
    }

    // Verify that all specified locations are scrubbed and locations outside
    // the requested are are not touched.
    #[test]
    fn test_touch_zero() {
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = 0;
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_one() {
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = cacheline_size;
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_many() {
        const MANY: usize = 50;
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = cacheline_size * MANY;
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_all() {
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = cacheline_size * TOUCHING_SANDBOX_SIZE;
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_double_all() {
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = 2 * cacheline_size * TOUCHING_SANDBOX_SIZE;
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_more_many() {
        const MANY: usize = 72;
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = 5 * cacheline_size * (TOUCHING_SANDBOX_SIZE + MANY);
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_touch_multiple_areas() {
        const MANY: usize = 72;
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = 2 * cacheline_size * (TOUCHING_SANDBOX_SIZE + MANY);
        let second_area = cacheline_size * TOUCHING_SANDBOX_SIZE;
        let third_area = cacheline_size * MANY;
        let scrub_areas = [first_area, second_area, third_area];
        test_scrubber(&scrub_areas, first_area);
    }

    #[test]
    fn test_big() {
        const MEM_AREA_SIZE: usize = 1 * 1024 * 1024 * 1024;

        let mut basic_cache_desc = BASIC_CACHE_DESC.clone();
        let cache_desc = &mut basic_cache_desc;
        let mem =
            match Mem::new::<BasicCacheline>(MEM_AREA_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        let scrub_areas = [mem.scrub_area];
        let mut scrubber =
            match MemoryScrubber::<BasicCacheDesc, BasicCacheline>::
            new(cache_desc, &scrub_areas) {
            Err(e) => panic!("Could not create MemoryScrubber: {}",
                e),
            Ok(scrubber) => scrubber,
        };

        // Use the first scrub to page in all memory
        match scrubber.scrub(MEM_AREA_SIZE) {
            Err(e) => panic!("Scrub failed: {}", e),
            Ok(_) => {},
        }

        println!("Please wait while timing scrub operation");
        let start_time = Instant::now();

        match scrubber.scrub(MEM_AREA_SIZE) {
            Err(e) => panic!("Scrub failed: {}", e),
            Ok(_) => {},
        }

        let end_time = start_time.elapsed();
        let duration = end_time.as_secs_f64();

        let mem_size = (MEM_AREA_SIZE as f64) / 1e9;
        println!("Scrub rate: {:.2} GBps", mem_size / duration);
    }

    #[test]
    fn test_autoscrub() {
        const CACHELINE_SIZE: usize = std::mem::size_of::<TouchingCacheline>();
        const ONE_SIZE: usize = TOUCHING_CACHE_LINES *
            TOUCHING_CACHE_NUM_TOUCHED * CACHELINE_SIZE;
        const SINGLE_SCAN: usize = ONE_SIZE / 2;
        const TOTAL_SCAN: usize = SINGLE_SCAN * 4 + 3 * CACHELINE_SIZE;

        let sizes = [ONE_SIZE, ONE_SIZE, ONE_SIZE];
        let (touching_cache_desc, scrub_areas) =
            setup_touching_desc_areas(&sizes);
        let cache_desc = &mut *touching_cache_desc.borrow_mut() as
            &mut TouchingCacheDesc;
        struct TestAutoScrubDesc {
            count: usize,
            scrub_size: usize,
        }

        impl AutoScrubDesc for TestAutoScrubDesc {
            fn next(&mut self) -> usize {
                let n = if self.count > self.scrub_size { self.scrub_size }
                    else { self.count };
                self.count -= n;
                n
            }
        }

        let mut autoscrub_desc = TestAutoScrubDesc {
            count: TOTAL_SCAN,
            scrub_size: SINGLE_SCAN,
        };

        let mut autoscrub = match AutoScrub::new(cache_desc, &scrub_areas,
            &mut autoscrub_desc) {
            Err(e) => panic!("AutoScrub::new failed: {}", e),
            Ok(autoscrub) => autoscrub,
        };
        match autoscrub.autoscrub() {
            Err(e) => panic!("autoscrub() failed: {}", e),
            Ok(_) => {},
        };
        verify_scrub(&autoscrub.scrubber, TOTAL_SCAN);
    }

    // Test support function that scrubs a section of memory, then verifies that
    // things were properly referred.
    // sizes - array of sizes of memory areas to scrub
    // n - number of cache lines to scrub
    fn test_scrubber(sizes: &[usize], n: usize) {
        let (touching_cache_desc, scrub_areas) =
            setup_touching_desc_areas(sizes);
        let cache_desc = &mut *touching_cache_desc.borrow_mut() as
            &mut TouchingCacheDesc;

        let mut memory_scrubber = {
            match MemoryScrubber::<TouchingCacheDesc, TouchingCacheline>
                ::new(cache_desc, &scrub_areas) {
                Err(e) => panic!("MemoryScrubber::new() failed {}", e),
                Ok(scrubber) => scrubber,
            }
        };

        if let Err(e) = memory_scrubber.scrub(n) {
            panic!("scrub failed: {}", e);
        };

        verify_scrub(&memory_scrubber, n);
    }

    // Set up a TouchingCacheDesc and ScrubAreas
    fn setup_touching_desc_areas (sizes: &[usize]) ->
        (Rc<RefCell<TouchingCacheDesc>>, Vec<ScrubArea>) {
        let touching_cache_desc =
            TouchingCacheDesc::new::<TouchingCacheline>(sizes);
        let touching_cache_desc = Rc::new(RefCell::new(touching_cache_desc));

        // Allocate memory areas according to the given sizes
        let mut scrub_areas: Vec<ScrubArea> = vec!();
        {
            let cache_desc = touching_cache_desc.borrow();
            let read_infos = cache_desc.read_infos.as_ref().unwrap();
            for read_info in read_infos {
                scrub_areas.push(read_info.mem.scrub_area.clone());
            }
        }

        (touching_cache_desc, scrub_areas)
    }

    // Verify the proper locations were hit
    // memory_scrubber - MemoryScrubber to use
    // touching_cache_desc - The TouchingCacheDesc to use.
    // n - bytes scrubbed
    //
    // This essentially reimplements the iterator code but in a more straight-
    // forward way so that the two implements can verify each other.
    fn verify_scrub(memory_scrubber: &MemoryScrubber<TouchingCacheDesc, TouchingCacheline>,
        n: usize) {
        let cacheline_width =
            memory_scrubber.cache_desc.borrow().cacheline_width();
        let n_in_cachelines = n >> cacheline_width;

        // Count the total number of scrub lines in all of the ScrubAreas
        let mut scrub_lines = 0;

        for scrub_area in memory_scrubber.scrub_areas {
            scrub_lines +=
                memory_scrubber.cache_desc.borrow()
                .size_in_cachelines(scrub_area);
        }

        let n_min_reads: NRead = match (n_in_cachelines / scrub_lines)
            .try_into() {
            Err(e) => panic!("Internal Error: n_min_reads conversion failed: {}", e),
            Ok(n_min_reads) => n_min_reads,
        };
        let n_extra_reads = n_in_cachelines % scrub_lines;

        let mut verified = 0;

        let scrubber = memory_scrubber.cache_desc.borrow();
        let read_infos = scrubber.read_infos.as_ref().unwrap();
        for read_info in read_infos {
            verify_read_info(&memory_scrubber, &read_info,
                n_min_reads, n_extra_reads, verified);
            verified += memory_scrubber.cache_desc.borrow()
                    .size_in_cachelines(&read_info.mem.scrub_area);
        }
    }

    // Verify that one scrub area is correct
    // memory_scrubber: The MemoryScrubber being verified
    // read_info: A ReadInfo being verified
    // n_min_reads: The minimum number of reads in the vectors in n_reads
    // n_extra_reads: The number of items in the vectors in n_reads which
    //      are greater than n_min_reads by one
    // verified: The number of items in the vectors of n_reads that have already
    //       been verified
    fn verify_read_info<'a>(memory_scrubber: &MemoryScrubber<TouchingCacheDesc, TouchingCacheline>,
        read_info: &ReadInfo, n_min_reads: NRead, n_extra_reads: usize,
        mut verified: usize) {
        let cache_desc = memory_scrubber.cache_desc.borrow();
        let cache_lines = {
            cache_desc.cache_lines()
        };
        let scrub_area = &read_info.mem.scrub_area;
        let scrub_lines =
            memory_scrubber.cache_desc.borrow().size_in_cachelines(scrub_area);
        let verified_end = verified + scrub_lines;

        let n_reads = &read_info.n_reads.as_ref().unwrap();
        verify_guard(n_reads, 0);

        // Now verify the contents of the memory to see whether they were
        // touched the expected number of times. The number of hits for a
        // location i in n_reads[] will be at least equal to the number of
        // complete scans of the memory area. Then, the remaining number of
        // items in the scan will be one larger.
        for line in 0..cache_lines {
            for i in (line..scrub_lines).step_by(cache_lines) {
                let inc = if verified < n_extra_reads { 1 } else { 0 };
                let expected: NRead = n_min_reads + inc;
                let actual = n_reads[GUARD_LINES + i];
                if actual != expected {
                    println!("verified {} n_extra_reads {} n_min_reads {}",
                        verified, n_extra_reads, n_min_reads);
                }
                assert_eq!(actual, expected as u8);
                verified += 1;
                if verified == verified_end {
                    return;
                }
            }
        }

        verify_guard(n_reads, GUARD_LINES + TOUCHING_SANDBOX_SIZE);
    }

    // Verify a guard area before the memory area. This should
    // not have been seen and so should have a zero value
    // n_reads - Array that should have all zero values at the offset for
    //      GUARD_LINES elements
    // offset - Offset in n_reads to check
    fn verify_guard(n_reads: &Vec<NRead>, offset: usize) {

        for i in 0..GUARD_LINES {
            let actual = n_reads[offset + i];
            assert_eq!(actual, 0);
        }
    }
}
