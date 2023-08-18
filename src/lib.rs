// This is code for a memory scrubber.
//
// What is a memory scrubber and why would you use one?
//
// A memory scrubber is simply a piece of hardware or software that reads all
// bytes from a section of memory, usually from all of memory. This is an
// implementation of a software memory scrubber. When a processor reads from
// memory protected by an error correction code (ECC), it checks to see if
// there are errors in the piece of memory it has read. If so, in hardware or
// software, the ECC is used to correct the errors and the corrected value
// used to replace the bad value.
//
// ECCs are limited in the number of errors they can correct. These errors
// generally accumulate over time. So long as memory is read often enough,
// correction is applied with enough frequency that the number of errors
// stays within the bounds of what is correctable. However, a piece of
// memory that is rarely accessed can accumulate multiple errors. When that
// memory is eventually used, it will not be possible to determine the corrected
// value and a fatal error will result. This is where a memory scrubber comes
// in.
//
// In general, memory is scrubbed at a rate high enough that the number of
// accumulated errors remains low enough that the number of uncorrectable
// errors is extremely low. Since it isn't possible to predict which areas of
// memory are read frequently enough to avoid error accumulation, the usual
// practice is to scan all of memory. With modern systems, this can be quite be
// a large amount of work and the scrubbing work is broken into smaller pieces
// to avoid any significant amount of performance impact.
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
// To use this, it recommended you do the following:
//
// 1.   Determine a suitable data type to represent the size of object that
//      is used by the unit the computes the ECC. This is probably either u32
//      or u64, and we'll call it ECCData here, though you can call it anything
//      you wish.
//
// 2.   Define the structure of a cache line by implementing Cacheline for
//      the particular layout for your processor. We'll call the structure
//      MyCacheline. It usually the case that cache lines are arrays of ECCData
//      items, such as:
//
//          type MyCacheline = [u64; 8];
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
//      an implementation of the CacheDesc trait. For example purposes, call
//      this MyCacheDesc. In most cases, the default functions provide
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
//          any element is read, this could be:
//
//              fn read_cacheline(&mut self, cacheline: *const Cacheline) {
//                  let _dummy = unsafe {
//                      ptr::read(&((*cacheline)[0]) as *const _);
//                  };
//              }
//
//          It is possible that the longest cache line will not be entirely
//          read when a single element is read. Since any memory not read
//          will not be checked for errors, it is important that this function
//          implement a full cache line read. Check your processor's reference
//          manual.
//
// 4.   Create a new MemoryScrubber:
//
//          let scrubber = match MemoryScrubber::<MyCacheline>::
//              new(&MyCacheDesc::<MyCacheline> {...}, my_start, my_end) {
//              Err(e) => ...
//
// 5.   Scrub some number of bytes. You could scrub a quarter of the memory area
//      with:
//
//          match scrubber.scrub(size / 4) {
//              Err(e) => ...
//
//      The size passed to scrub_scrub_areIa() must be a multiple of the cache line size.

use std::cell::RefCell;
use std::iter;
use std::rc::Rc;
use thiserror::Error;

// Data type that can hold any address for manipulation as an integer
type Addr = usize;

// Describe cache parameters and pull in all elements of the cache line.
pub trait CacheDesc<Cacheline> {
    // NOTE: You are unlikely to ever need to implement this
    // Return the number of bits required to hold an index into the bytes of
    // a cacheline. So, if you have an eight-byte cache line (unlikely), this
    // would return 3.
    fn cacheline_width(&self) -> usize {
        usize::BITS as usize - 1 - std::mem::size_of::<Cacheline>()
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
    fn read_cacheline(&mut self, p: *const Cacheline);

    // Return the size of a ScrubArea in cachelines
    fn size_in_cachelines(&self, scrub_area: &ScrubArea) -> usize {
        let start_in_cachelines =
            scrub_area.start as usize >> self.cacheline_width();
        // This will truncate the number of cache lines by one
        let end_in_cachelines =
            scrub_area.end as usize >> self.cacheline_width();
        (end_in_cachelines - start_in_cachelines) + 1
    }
}

pub type CacheDescRc<'a, Cacheline> =
    Rc<RefCell<&'a mut dyn CacheDesc<Cacheline>>>;

#[derive(Clone, Copy, Debug, Error, PartialEq)]
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

    #[error("std::mem::size_of::<Cacheline> should equal CacheDescRc.cacheline_size()")]
    CachelineSizeMismatch,

    #[error("Internal Error: Iterator failed")]
    IteratorFailed,
}

// Memory scrubber
// cache_desc - Description of the cache
// scrub_areas - ScrubAreas being scrubbed
// iterator - MemoryScrubberIterator used to walk through the memory being
//      scrubbed
pub struct MemoryScrubber<'a, Cacheline> {
    cache_desc:     CacheDescRc<'a, Cacheline>,
    scrub_areas:    &'a [ScrubArea],
    iterator:       Option<MemoryScrubberIterator<'a, Cacheline>>,
}

impl<'a, Cacheline> MemoryScrubber<'a, Cacheline> {

    // Create a new memory scrubber
    // cache_desc - Description of the cache
    // start - Virtual address of memory being scrubbed
    // end - Virtual address of the last byte of memory to be scrubbed
    pub fn new(cache_desc: &'a mut dyn CacheDesc<Cacheline>,
        scrub_areas: &'a [ScrubArea]) ->
        Result<MemoryScrubber<'a, Cacheline>, Error> {

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

        Ok(MemoryScrubber::<'a> {
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
            return Err(Error::UnalignedSize);
        }

        // Convert to the number of cachelines to scrub
        let cachelines_to_scrub = n >> cacheline_width;

        for _i in 0..cachelines_to_scrub {
            // If we don't have an iterator, get one
            if self.iterator.is_none() {
                self.iterator =
                    Some(MemoryScrubberIterator::new(self.cache_desc.clone(),
                    &self.scrub_areas));
            }
            let next = self.iterator.as_mut().unwrap().next();

            match next {
                None => self.iterator = None,
                Some(p) => {
                    self.cache_desc.borrow_mut().read_cacheline(p);
                },
            };
        }

        
        Ok(())
    }
}

pub struct MemoryScrubberIterator<'b, Cacheline> {
    cache_desc:     CacheDescRc<'b, Cacheline>,
    scrub_areas:    &'b [ScrubArea],
    iterator:       Option<ScrubAreaIterator<'b, Cacheline>>,
    index:          usize,
}

impl <'b, Cacheline> MemoryScrubberIterator<'b, Cacheline> {
    pub fn new(cache_desc: CacheDescRc<'b, Cacheline>,
        scrub_areas: &'b [ScrubArea]) ->
        MemoryScrubberIterator<'b, Cacheline> {

        MemoryScrubberIterator {
            cache_desc:     cache_desc,
            scrub_areas:    scrub_areas,
            iterator:       None,
            index:          0,
        }
    }
}

impl<Cacheline> iter::Iterator for MemoryScrubberIterator<'_, Cacheline> {
    type Item = *const Cacheline;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.index == self.scrub_areas.len() {
                return None;
            }

            if self.iterator.is_none() {
                self.iterator =
                    ScrubAreaIterator::new(self.cache_desc.clone(),
                    &self.scrub_areas[self.index]);
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
pub struct ScrubAreaIterator<'b, Cacheline> {
    cache_desc: CacheDescRc<'b, Cacheline>,
    scrub_area: ScrubArea,
    index:      usize,
    offset:     usize,
}

impl<'b, Cacheline> ScrubAreaIterator<'b, Cacheline> {
    // Create a new ScrubAreaIterator.
    // scrub_area: Memory over which we Iterate
    //
    // Returns: Ok(ScrubAreaIterator) on success, Err(Error) on failure
    pub fn new(cache_desc: CacheDescRc<'b, Cacheline>,
        scrub_area: &ScrubArea) ->
        Option<ScrubAreaIterator<'b, Cacheline>> {

        Some(ScrubAreaIterator {
            cache_desc: cache_desc,
            scrub_area: scrub_area.clone(),
            index:      0,
            offset:     0,
        })
    }
}

// Return a pointer into a series of Cacheline items. To get a byte address
// from the return value of next(), call it ret_val, use:
impl<Cacheline> iter::Iterator for ScrubAreaIterator<'_, Cacheline> {
    type Item = *const Cacheline;

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
            let start_index = cache_index::<Cacheline>(&self.cache_desc,
                self.scrub_area.start);
            let offset = start_index + self.index + self.offset;
            let size = self.cache_desc.borrow().size_in_cachelines(&self.scrub_area);

            if offset < size {
                let res = unsafe {
                    (self.scrub_area.start as *const Cacheline)
                        .offset(offset as isize)
                };
                self.offset += self.cache_desc.borrow().cache_lines();
                return Some(res as *const Cacheline);
            }
            self.index += 1;
            self.offset = 0;
        }
    }
}

// Structure used to define an area to be scribbed
// start - lowest address of the area. Must be a multiple of the cache line size
// end - address of the last byte of the area. Must be one less than a multiple
//      of the cache line size
#[derive(Clone, Debug)]
pub struct ScrubArea {
    start:              *const u8,
    end:                *const u8,
}

impl ScrubArea {
    pub fn new(start: *const u8, end: *const u8) -> ScrubArea {
        ScrubArea {
            start: start,
            end: end
        }
    }
}


// Returns the cache index part of the address
fn cache_index<Cacheline>(cache_desc: &CacheDescRc<Cacheline>, p: *const u8) ->
    usize {
    (p as Addr) >> cache_desc.borrow().cacheline_width() &
        ((1 << cache_desc.borrow().cache_index_width()) - 1)
}

#[cfg(test)]
mod tests {
    //use std::cell::RefCell;
    use std::ptr;
    //use std::rc::Rc;
    use std::time::Instant;

    use crate::{Addr, CacheDesc, Error, MemoryScrubber, ScrubArea};

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
    type BasicCacheline = [BasicECCData;
        (1 << BASIC_CACHELINE_WIDTH) / std::mem::size_of::<BasicECCData>()];

    // BasicTestCacheDescRc - Description of the cache for basic tests
    // cache_index_width - Number of bits of the cache index.
    #[derive(Clone, Copy, Debug)]
    struct BasicTestCacheDescRc {
        cache_index_width:        usize,
    }

    // Cache descriptor to pass to the memory scrubbing functions.
    static BASIC_CACHE_DESC: BasicTestCacheDescRc = BasicTestCacheDescRc {
        cache_index_width:        BASIC_CACHE_INDEX_WIDTH,
    };

    impl CacheDesc<BasicCacheline> for BasicTestCacheDescRc {
        fn cache_index_width(&self) -> usize {
            self.cache_index_width
        }

        fn read_cacheline(&mut self, cacheline: *const BasicCacheline) {
            let _dummy = unsafe {
                ptr::read(&((*cacheline)[0]) as *const _)
            };
        }
    }

    // Cache characteristics
    // TOUCHING_CACHELINE_WIDTH - number of bits required to index a byte in a
    //  cache line
    // TOUCHING_CACHE_INDEX_WIDTH - number of bits used as a cache line index
    //  in the cache
    // TOUCHING_CACHE_LINES - number of cache lines
    const TOUCHING_ECCDATA_WIDTH: usize = usize::BITS as usize - 1 -
        std::mem::size_of::<TouchingECCData>() .leading_zeros() as usize;
    const TOUCHING_CACHELINE_WIDTH: usize = 3 + TOUCHING_ECCDATA_WIDTH;
    const TOUCHING_CACHELINE_SIZE: usize = 1 << TOUCHING_CACHELINE_WIDTH;
/* FIXME: restore to this value
    const TOUCHING_CACHE_INDEX_WIDTH: usize = 10;
*/
    const TOUCHING_CACHE_INDEX_WIDTH: usize = 2;
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

    // TouchingCacheline - the data type of a cache line
    type TouchingCacheline = [TouchingECCData;
        TOUCHING_CACHELINE_SIZE / std::mem::size_of::<TouchingECCData>()];

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
        fn new<Cacheline>(cacheline_size: usize, size: usize) ->
            Result<Mem, Error> {
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
println!("start {:x} size {} end {:x}", start_addr, size, end as usize);

            Ok(Mem {
                mem_area:   mem_area,
                scrub_area: ScrubArea::new(start, end),
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
        fn new(sizes: &[usize]) -> TouchingCacheDesc {
            // FIXME: clean this up
            let mut touching_cache_desc = TOUCHING_CACHE_DESC.clone();

            let cacheline_size = touching_cache_desc.cacheline_size();
            let mut read_infos = vec!();

            for size in sizes {
                // Can I eliminate touching_cache_desc from here?
                let mem =
                    match Mem::new::<TouchingCacheline>(cacheline_size, *size) {
                    Err(e) => panic!("Memory allocation error: {}", e),
                    Ok(mem) => mem,
                };

                let n_reads_size =
                    touching_cache_desc.size_in_cachelines(&mem.scrub_area);
                read_infos.push(ReadInfo::new(n_reads_size, mem));
            }

            touching_cache_desc.read_infos = Some(read_infos);
            touching_cache_desc
        }
    }

    impl CacheDesc<TouchingCacheline> for TouchingCacheDesc {
        fn cache_index_width(&self) -> usize {
            self.cache_index_width
        }

        fn read_cacheline(&mut self, cacheline: *const TouchingCacheline) {
            // First, actually do the read
            let _dummy = unsafe {
                ptr::read(&(*cacheline)[0]);
            };
            let cacheline_size = self.cacheline_size ();

            // Find the corresponding location and update the number of reads
            for read_info in &mut self.read_infos.as_mut().unwrap().iter_mut() {
                let cacheline_addr = cacheline as Addr;
                let start_addr = read_info.mem.scrub_area.start as Addr;

                if cacheline_addr >= start_addr &&
                    cacheline_addr <= read_info.mem.scrub_area.end as Addr {
                    let offset = (cacheline_addr - start_addr) / cacheline_size;
                    let n_reads = &mut read_info.n_reads.as_mut().unwrap();
                    n_reads[offset] += 1;
println!("read_cacheline: offset {} n_reads[{}] {}", offset, offset, n_reads[offset]);
                }
            }
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
        let cacheline_size = basic_cache_desc.cacheline_size();
        let mut mem =
            match Mem::new::<BasicCacheline>(cacheline_size,
            BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.start = unsafe {
            mem.scrub_area.start.offset(1)
        };

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheline>::new(basic_cache_desc,
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
        let cacheline_size = basic_cache_desc.cacheline_size();
        let mut mem =
            match Mem::new::<BasicCacheline>(cacheline_size,
            BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.end = unsafe {
            mem.scrub_area.end.offset(-1)
        };

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheline>::new(basic_cache_desc,
            &scrub_areas);
        assert!(memory_scrubber.is_err());
        assert_eq!(memory_scrubber.err().unwrap(),
            Error::UnalignedEnd);
    }

    // Verify that an error is returned if the size is zero.
    #[test]
    fn test_zero_size() {
        let basic_cache_desc = &mut BASIC_CACHE_DESC.clone();
        let cacheline_size = basic_cache_desc.cacheline_size();
        let mut mem =
            match Mem::new::<BasicCacheline>(cacheline_size,
            BASIC_MEM_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        mem.scrub_area.end = mem.scrub_area.start;

        let scrub_areas = [mem.scrub_area];
        let memory_scrubber =
            MemoryScrubber::<BasicCacheline>::new(basic_cache_desc,
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
        let mut mem =
            match Mem::new::<BasicCacheline>(cacheline_size,
            basic_cache_desc.cacheline_size() *
            basic_cache_desc.cache_lines() * 14) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };

        let scrub_areas = [mem.scrub_area];
        let mut memory_scrubber =
            match MemoryScrubber::<BasicCacheline>::new(basic_cache_desc,
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

    // Test scrubbing:
    // o    Zero cache lines
    // o    One cache line
    // o    Fifty cache lines
    // o    The entire size of the cache area
    // o    Double the cache area size plus fifty (test wrapping)
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
    fn test_touch_many_many() {
        const MANY: usize = 72;
        let cacheline_size = TOUCHING_CACHE_DESC.cacheline_size();
        let first_area = 5 * cacheline_size * (TOUCHING_SANDBOX_SIZE + MANY);
        test_scrubber(&[cacheline_size * TOUCHING_SANDBOX_SIZE], first_area);
    }

    #[test]
    fn test_big() {
        const MEM_AREA_SIZE: usize = 1 * 1024 * 1024 * 1024;

        let mut basic_cache_desc = BASIC_CACHE_DESC.clone();
        let cacheline_size = basic_cache_desc.cacheline_size();
        let cache_desc = &mut basic_cache_desc;
        let mut mem =
            match Mem::new::<BasicCacheline>(cacheline_size,
            MEM_AREA_SIZE) {
            Err(e) => panic!("Memory allocation error: {}", e),
            Ok(mem) => mem,
        };
        let sizes = [mem.scrub_area];
        let mut scrubber = match MemoryScrubber::<BasicCacheline>::
            new(cache_desc, &sizes) {
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

    // Test support function that scrubs a section of memory, then verifies that
    // things were properly referred.
    // sizes - array of sizes of memory areas to scrub
    // n - number of cache lines to scrub
    fn test_scrubber(sizes: &[usize], n: usize) {
        let touching_cache_desc = TouchingCacheDesc::new(&sizes);
        let cache_desc = &mut touching_cache_desc.clone() as &mut dyn CacheDesc<_>;
        let cacheline_width = cache_desc.cacheline_width();

        // Allocate memory areas according to the given sizes
        let mut scrub_areas: Vec<ScrubArea> = vec!();
        for read_info in touching_cache_desc.clone().read_infos
            .as_ref().unwrap() {
            scrub_areas.push(read_info.mem.scrub_area.clone());
        }

println!("scrub_areas {:?}", scrub_areas);
        let mut memory_scrubber =
            match MemoryScrubber::new(cache_desc, &scrub_areas) {
            Err(e) => panic!("MemoryScrubber::new() failed {}", e),
            Ok(scrubber) => scrubber,
        };

        if let Err(e) = memory_scrubber.scrub(n) {
            panic!("scrub failed: {}", e);
        };

        verify_scrub(memory_scrubber, &touching_cache_desc,
            n >> cacheline_width);

        // This is used to keep mem_area from being deallocated before we're
        // done
        for read_info in touching_cache_desc.read_infos.unwrap() {
            assert_ne!(read_info.mem.mem_area.len(), 0);
        }
    }

    // Verify the proper locations were hit
    // memory_scrubber - MemoryScrubber to use
    // n - number of cache lines scrubbed
    //
    // WARNING: So, you're a super hot shit programmer who's been reading the
    // code above and you thing to yourself, "hey, this is just like the
    // Iterators, why don't I switch this to use them?" Don't. This is a
    // deliberate reimplementation of what the Iterators are doing, but without
    // the Iterators because the Iterators are what is being tested. It is
    // thus an independent implementation for testing purposes.
    fn verify_scrub(memory_scrubber: MemoryScrubber<TouchingCacheline>,
        touching_cache_desc: &TouchingCacheDesc, n: usize) {
println!(">>>> Verifying");
        let _cacheline_width =
            memory_scrubber.cache_desc.borrow().cacheline_width();

        // Count the total number of scrub lines in all of the ScrubAreas
        let mut scrub_lines = 0;

        for scrub_area in memory_scrubber.scrub_areas {
            scrub_lines +=
                memory_scrubber.cache_desc.borrow()
                .size_in_cachelines(scrub_area);
        }

        let n_min_reads = n / scrub_lines;
        let n_extra_reads = n % scrub_lines;

        let mut verified = 0;

        for read_info in touching_cache_desc.read_infos.as_ref().unwrap() {
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
    fn verify_read_info<'b>(memory_scrubber: &MemoryScrubber<TouchingCacheline>,
        read_info: &ReadInfo, n_min_reads: usize, n_extra_reads: usize,
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
println!("verify_read_info: n_min_reads {} n_extra_reads {}", n_min_reads, n_extra_reads);
        for line in 0..cache_lines {
            for i in (line..scrub_lines).step_by(cache_lines) {
                let inc = if verified < n_extra_reads { 0 } else { 1 };
                let expected = n_min_reads + inc;
println!("verify_read_info: verified {} line {} i {} n_reads[{}] {}", verified, line, i, GUARD_LINES + i, n_reads[GUARD_LINES + 1]);
                let actual = n_reads[GUARD_LINES + i];
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
