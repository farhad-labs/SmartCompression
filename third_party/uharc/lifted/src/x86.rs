//! Execution state for the lifted UHARC codec engine (`src/engine.rs`).
//!
//! The engine is the original program's own compressor/decompressor,
//! translated instruction by instruction, so it needs the machine model the
//! instructions were written for: eight 32-bit registers, the arithmetic
//! flags, the direction flag and a flat little-endian 32-bit address space.
//! The address space holds the original's data segment at its link address
//! (`0x434000..`), its zero-initialised `.bss`, a stack below the image and
//! the blocks the engine obtains from `VirtualAlloc` above it.
//!
//! The engine talks to the rest of the program through a handful of calls
//! that leave it: the front end's read/write callbacks and four Win32
//! imports.  Those are implemented here, in Rust.

/// Register numbers, in x86 encoding order.
pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

/// Front-end callbacks the engine is handed as code addresses.  The values
/// are the addresses of the original routines, so the engine sees exactly the
/// pointers it saw in UHARC.EXE.
pub const CB_READ_TABLE: u32 = 0x405e20;
pub const CB_READ_DATA: u32 = 0x409a00;
pub const CB_WRITE: u32 = 0x405df0;
/// Extraction: archive reader, table writer and file-data writer.
pub const CB_READ_ARCHIVE: u32 = 0x405d90;
pub const CB_WRITE_TABLE: u32 = 0x4066e0;
pub const CB_WRITE_FILES: u32 = 0x40b170;
/// The front end's progress display, called back by the engine with the
/// number of bytes done; it returns AL != 0 to abort.
pub const CB_PROGRESS: u32 = 0x407ab0;
/// The runtime's lock/unlock hooks point at a bare `RET`.
const HOOK_NOP: u32 = 0x424734;

/// Top of the emulated stack (it grows down, below the image).
pub const STACK_TOP: u32 = 0x003F_F000;
/// The return address handed to an engine entry point; `RET` to it ends the call.
pub const EXIT_ADDR: u32 = 0xFFFF_FFF0;
/// First address handed out by `VirtualAlloc`.
const HEAP_BASE: u32 = 0x0100_0000;

/// Where the original's data segment lives, and its initial contents.
pub const DGROUP_BASE: u32 = 0x434000;
static DGROUP: &[u8] = include_bytes!("engine_dgroup.bin");
/// Constant tables the original keeps in its code section (no instructions):
/// repeated `[u32 address][u32 length][bytes]`.
static CODE_DATA: &[u8] = include_bytes!("engine_codedata.bin");
/// End of `.bss` (`0x48a000`).
pub const IMAGE_END: u32 = 0x48A000;

/// Address of the lifted basic block being executed; only written by an
/// engine generated with `LIFT_PROF=1`, sampled by `start_sampler`.
pub static PROF_PC: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Profiling aid: sample `PROF_PC` every ~100us and write a histogram to
/// `path` when the returned guard is dropped.
pub struct Sampler {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    th: Option<std::thread::JoinHandle<std::collections::HashMap<u32, u64>>>,
    path: String,
}
pub fn start_sampler(path: String) -> Sampler {
    use std::sync::atomic::Ordering::Relaxed;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s2 = stop.clone();
    let th = std::thread::spawn(move || {
        let mut h = std::collections::HashMap::new();
        while !s2.load(Relaxed) {
            *h.entry(PROF_PC.load(Relaxed)).or_insert(0u64) += 1;
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
        h
    });
    Sampler { stop, th: Some(th), path }
}
impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let h = self.th.take().unwrap().join().unwrap();
        let mut v: Vec<_> = h.into_iter().collect();
        v.sort();
        let s: String = v.iter().map(|(a, n)| format!("{a:x} {n}\n")).collect();
        let _ = std::fs::write(&self.path, s);
    }
}

/// The guest's flat 32-bit address space.
///
/// On Windows the whole 4 GiB (plus a guard) is *reserved* up front and pages
/// are committed as the engine allocates them, so every possible guest
/// address lies inside the reservation.  The lifted code can then skip
/// per-access bounds checks (`Mem`): an access to an address the engine never
/// allocated hits an inaccessible page and faults instead of touching host
/// memory.  Committed pages are zero-filled by the OS, like `VirtualAlloc`
/// in the original.  Elsewhere it is a plain `Vec` and `Mem` keeps checking.
pub struct GuestMem {
    #[cfg(windows)]
    base: *mut u8,
    #[cfg(windows)]
    len: usize,
    #[cfg(not(windows))]
    v: Vec<u8>,
}

#[cfg(windows)]
mod win {
    extern "system" {
        pub fn VirtualAlloc(addr: *mut u8, size: usize, alloc_type: u32, protect: u32) -> *mut u8;
        pub fn VirtualFree(addr: *mut u8, size: usize, free_type: u32) -> i32;
    }
    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;
    pub const PAGE_NOACCESS: u32 = 1;
    pub const PAGE_READWRITE: u32 = 4;
}

#[cfg(windows)]
impl GuestMem {
    /// 4 GiB of guest addresses plus 64 KiB so that a 4-byte access starting
    /// at 0xFFFF_FFFF stays inside the (inaccessible) reservation.
    const RESERVE: usize = 0x1_0001_0000;

    pub fn new(len: usize) -> Self {
        let base = unsafe { win::VirtualAlloc(std::ptr::null_mut(), Self::RESERVE, win::MEM_RESERVE, win::PAGE_NOACCESS) };
        assert!(!base.is_null(), "cannot reserve the engine's 4 GiB address space");
        let mut m = GuestMem { base, len: 0 };
        m.resize(len, 0);
        m
    }
    /// Commit guest memory up to `new_len` bytes (never shrinks).
    pub fn resize(&mut self, new_len: usize, _fill: u8) {
        if new_len <= self.len {
            return;
        }
        let start = self.len & !0xFFF;
        let p = unsafe { win::VirtualAlloc(self.base.add(start), new_len - start, win::MEM_COMMIT, win::PAGE_READWRITE) };
        assert!(!p.is_null(), "out of memory committing {new_len:#x} bytes of engine memory");
        self.len = new_len;
    }
}
#[cfg(windows)]
impl Drop for GuestMem {
    fn drop(&mut self) {
        unsafe { win::VirtualFree(self.base, 0, win::MEM_RELEASE) };
    }
}
#[cfg(windows)]
impl std::ops::Deref for GuestMem {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // `base..base+len` is committed read/write memory owned by `self`.
        unsafe { std::slice::from_raw_parts(self.base, self.len) }
    }
}
#[cfg(windows)]
impl std::ops::DerefMut for GuestMem {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.base, self.len) }
    }
}

#[cfg(not(windows))]
impl GuestMem {
    pub fn new(len: usize) -> Self {
        GuestMem { v: vec![0; len] }
    }
    pub fn resize(&mut self, new_len: usize, fill: u8) {
        if new_len > self.v.len() {
            self.v.resize(new_len, fill);
        }
    }
}
#[cfg(not(windows))]
impl std::ops::Deref for GuestMem {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.v
    }
}
#[cfg(not(windows))]
impl std::ops::DerefMut for GuestMem {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.v
    }
}

pub struct Cpu {
    pub r: [u32; 8],
    pub cf: bool,
    pub zf: bool,
    pub sf: bool,
    pub of: bool,
    pub df: bool,
    pub mem: GuestMem,
    heap_next: u32,
    /// Data the read callback hands to the engine.
    pub input: Vec<u8>,
    pub in_pos: usize,
    /// Bytes the engine passed to the write callback.
    pub output: Vec<u8>,
    /// The tick count `GetTickCount` reports; it only paces the progress
    /// display.
    pub ticks: u32,
    /// Files opened through the host `fopen`; a `FILE *` is `FILE_BASE + 16*i`.
    files: Vec<Option<std::fs::File>>,
    /// Rust side of the file-data write callback used while extracting.
    pub write_hook: Option<Box<dyn FnMut(&mut Cpu, u32, u32) -> u32>>,
    /// What the host `fwrite` collects (the file being extracted).
    pub fwrite_sink: Vec<u8>,
    /// Rust side of the display callback: (event in AL, EDX, EBX) -> abort?
    pub progress_hook: Option<Box<dyn FnMut(&mut Cpu, u32, u32, u32) -> bool>>,
    /// Released `VirtualAlloc` blocks, (base, size), reused first-fit.
    free_blocks: Vec<(u32, u32)>,
    probes: u32,
    /// Live `VirtualAlloc` blocks, base -> size.
    blocks: std::collections::HashMap<u32, u32>,
}

/// Fake `FILE *` values handed to the engine (it never dereferences them).
const FILE_BASE: u32 = 0x7F00_0000;

impl Cpu {
    pub fn new() -> Self {
        let mut mem = GuestMem::new(IMAGE_END as usize);
        let b = DGROUP_BASE as usize;
        mem[b..b + DGROUP.len()].copy_from_slice(DGROUP);
        let mut i = 0;
        while i < CODE_DATA.len() {
            let a = u32::from_le_bytes(CODE_DATA[i..i + 4].try_into().unwrap()) as usize;
            let n = u32::from_le_bytes(CODE_DATA[i + 4..i + 8].try_into().unwrap()) as usize;
            mem[a..a + n].copy_from_slice(&CODE_DATA[i + 8..i + 8 + n]);
            i += 8 + n;
        }
        let mut c = Cpu {
            r: [0; 8],
            cf: false,
            zf: false,
            sf: false,
            of: false,
            df: false,
            mem,
            heap_next: HEAP_BASE,
            input: Vec::new(),
            in_pos: 0,
            output: Vec::new(),
            ticks: 0,
            files: Vec::new(),
            free_blocks: Vec::new(),
            probes: 0,
            write_hook: None,
            fwrite_sink: Vec::new(),
            progress_hook: None,
            blocks: std::collections::HashMap::new(),
        };
        // The C runtime installs these during startup.
        c.wr32(0x43e2d8, HOOK_NOP);
        c.wr32(0x43e2e0, HOOK_NOP);
        c
    }

    // ----- memory -------------------------------------------------------

    #[inline]
    fn chk(&self, a: u32, n: usize) -> usize {
        let i = a as usize;
        if i + n > self.mem.len() {
            panic!("engine memory access out of range: {a:#x}");
        }
        i
    }
    #[inline]
    pub fn rd8(&self, a: u32) -> u32 {
        let i = self.chk(a, 1);
        self.mem[i] as u32
    }
    #[inline]
    pub fn rd16(&self, a: u32) -> u32 {
        let i = self.chk(a, 2);
        u16::from_le_bytes([self.mem[i], self.mem[i + 1]]) as u32
    }
    #[inline]
    pub fn rd32(&self, a: u32) -> u32 {
        let i = self.chk(a, 4);
        u32::from_le_bytes(self.mem[i..i + 4].try_into().unwrap())
    }
    #[inline]
    pub fn wr8(&mut self, a: u32, v: u32) {
        let i = self.chk(a, 1);
        self.mem[i] = v as u8;
    }
    #[inline]
    pub fn wr16(&mut self, a: u32, v: u32) {
        let i = self.chk(a, 2);
        self.mem[i..i + 2].copy_from_slice(&(v as u16).to_le_bytes());
    }
    #[inline]
    pub fn wr32(&mut self, a: u32, v: u32) {
        let i = self.chk(a, 4);
        self.mem[i..i + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn read_bytes(&self, a: u32, n: usize) -> &[u8] {
        let i = self.chk(a, n);
        &self.mem[i..i + n]
    }
    pub fn write_bytes(&mut self, a: u32, data: &[u8]) {
        let i = self.chk(a, data.len());
        self.mem[i..i + data.len()].copy_from_slice(data);
    }

    // ----- stack --------------------------------------------------------

    #[inline]
    pub fn push(&mut self, v: u32) {
        self.r[ESP] = self.r[ESP].wrapping_sub(4);
        let sp = self.r[ESP];
        self.wr32(sp, v);
    }
    #[inline]
    pub fn push16(&mut self, v: u32) {
        self.r[ESP] = self.r[ESP].wrapping_sub(2);
        let sp = self.r[ESP];
        self.wr16(sp, v);
    }
    #[inline]
    pub fn pop(&mut self) -> u32 {
        let v = self.rd32(self.r[ESP]);
        self.r[ESP] = self.r[ESP].wrapping_add(4);
        v
    }
    #[inline]
    pub fn pop16(&mut self) -> u32 {
        let v = self.rd16(self.r[ESP]);
        self.r[ESP] = self.r[ESP].wrapping_add(2);
        v
    }
    /// `RET n`: drop the return address and `n` bytes of arguments.
    #[inline]
    pub fn ret(&mut self, n: u32) {
        self.r[ESP] = self.r[ESP].wrapping_add(4 + n);
    }

    /// Registers only: the flags are not live across this call boundary
    /// (the lifter proved it), so they are neither stored nor loaded.
    /// `df` is cheap and stays exact.
    #[inline(always)]
    pub fn regs_nf(&self) -> Regs {
        Regs { r: self.r, cf: false, zf: false, sf: false, of: false, df: self.df }
    }
    #[inline(always)]
    pub fn set_regs_nf(&mut self, l: &Regs) {
        self.r = l.r;
        self.df = l.df;
    }
    /// Fresh memory view for a lifted function (see `Mem`).
    #[inline(always)]
    pub fn mem_view(&mut self) -> Mem {
        Mem { p: self.mem.as_mut_ptr(), len: self.mem.len() }
    }
    /// Snapshot of the register file for a lifted function (see `Regs`).
    #[inline(always)]
    pub fn regs(&self) -> Regs {
        Regs { r: self.r, cf: self.cf, zf: self.zf, sf: self.sf, of: self.of, df: self.df }
    }
    /// Publish a lifted function's registers (before a call or return).
    #[inline(always)]
    pub fn set_regs(&mut self, l: &Regs) {
        self.r = l.r;
        self.cf = l.cf;
        self.zf = l.zf;
        self.sf = l.sf;
        self.of = l.of;
        self.df = l.df;
    }
}

/// A function's view of the engine memory: base pointer and length held in
/// locals so accesses need no reload through `Cpu`.  It is only valid until
/// the next call out of the function (`VirtualAlloc` may grow and move the
/// memory), so lifted code takes a fresh view after every call.
#[derive(Clone, Copy)]
pub struct Mem {
    p: *mut u8,
    len: usize,
}

/// With the reserved 4 GiB guest space (`GuestMem`, Windows release builds)
/// no check is needed: every address is inside the reservation.
const CHECKED: bool = cfg!(debug_assertions) || cfg!(not(windows));

#[cold]
#[inline(never)]
fn oob(a: u32) -> ! {
    panic!("engine memory access out of range: {a:#x}")
}

impl Mem {
    // Checked builds compare against `len` (`a as usize + n` cannot wrap on a
    // 64-bit target); unchecked ones rely on the reservation described at
    // `GuestMem`.  Either way the pointer accesses below stay inside memory
    // owned by the `Cpu` the view was taken from.
    #[inline(always)]
    pub fn rd8(self, a: u32) -> u32 {
        if CHECKED && a as usize >= self.len {
            oob(a)
        }
        unsafe { *self.p.add(a as usize) as u32 }
    }
    #[inline(always)]
    pub fn rd16(self, a: u32) -> u32 {
        if CHECKED && a as usize + 2 > self.len {
            oob(a)
        }
        unsafe { (self.p.add(a as usize) as *const u16).read_unaligned() as u32 }
    }
    #[inline(always)]
    pub fn rd32(self, a: u32) -> u32 {
        if CHECKED && a as usize + 4 > self.len {
            oob(a)
        }
        unsafe { (self.p.add(a as usize) as *const u32).read_unaligned() }
    }
    #[inline(always)]
    pub fn wr8(self, a: u32, v: u32) {
        if CHECKED && a as usize >= self.len {
            oob(a)
        }
        unsafe { *self.p.add(a as usize) = v as u8 }
    }
    #[inline(always)]
    pub fn wr16(self, a: u32, v: u32) {
        if CHECKED && a as usize + 2 > self.len {
            oob(a)
        }
        unsafe { (self.p.add(a as usize) as *mut u16).write_unaligned(v as u16) }
    }
    #[inline(always)]
    pub fn wr32(self, a: u32, v: u32) {
        if CHECKED && a as usize + 4 > self.len {
            oob(a)
        }
        unsafe { (self.p.add(a as usize) as *mut u32).write_unaligned(v) }
    }
}

/// Registers and flags as a plain value.  Each lifted function works on a
/// local copy so the compiler can keep them in machine registers; the copy
/// is written back to the `Cpu` only around calls and returns.
#[derive(Clone, Copy)]
pub struct Regs {
    pub r: [u32; 8],
    pub cf: bool,
    pub zf: bool,
    pub sf: bool,
    pub of: bool,
    pub df: bool,
}

impl Regs {
    // ----- stack --------------------------------------------------------

    #[inline(always)]
    pub fn push(&mut self, m: Mem, v: u32) {
        self.r[ESP] = self.r[ESP].wrapping_sub(4);
        m.wr32(self.r[ESP], v);
    }
    #[inline(always)]
    pub fn push16(&mut self, m: Mem, v: u32) {
        self.r[ESP] = self.r[ESP].wrapping_sub(2);
        m.wr16(self.r[ESP], v);
    }
    #[inline(always)]
    pub fn pop(&mut self, m: Mem) -> u32 {
        let v = m.rd32(self.r[ESP]);
        self.r[ESP] = self.r[ESP].wrapping_add(4);
        v
    }
    #[inline(always)]
    pub fn pop16(&mut self, m: Mem) -> u32 {
        let v = m.rd16(self.r[ESP]);
        self.r[ESP] = self.r[ESP].wrapping_add(2);
        v
    }
    /// `RET n`: drop the return address and `n` bytes of arguments.
    #[inline(always)]
    pub fn ret(&mut self, n: u32) {
        self.r[ESP] = self.r[ESP].wrapping_add(4 + n);
    }

    // ----- flags --------------------------------------------------------

    #[inline(always)]
    fn zs(&mut self, r: u32, sign: u32) {
        self.zf = r == 0;
        self.sf = r & sign != 0;
    }
}

macro_rules! sized_ops {
    ($bits:expr, $add:ident, $sub:ident, $adc:ident, $sbb:ident, $and:ident, $or:ident,
     $xor:ident, $inc:ident, $dec:ident, $shl:ident, $shr:ident, $sar:ident, $ror:ident,
     $rol:ident, $imul:ident, $pshl:ident, $pshr:ident, $psar:ident, $pror:ident, $prol:ident,
     $sty:ty) => {
        impl Regs {
            #[inline(always)]
            pub fn $add(&mut self, a: u32, b: u32) -> u32 {
                const M: u64 = (1u64 << $bits) - 1;
                const S: u32 = 1u32 << ($bits - 1);
                let w = a as u64 + b as u64;
                let r = (w & M) as u32;
                self.cf = w > M;
                self.of = (a ^ r) & (b ^ r) & S != 0;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $adc(&mut self, a: u32, b: u32) -> u32 {
                const M: u64 = (1u64 << $bits) - 1;
                const S: u32 = 1u32 << ($bits - 1);
                let w = a as u64 + b as u64 + self.cf as u64;
                let r = (w & M) as u32;
                self.cf = w > M;
                self.of = (a ^ r) & (b ^ r) & S != 0;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $sub(&mut self, a: u32, b: u32) -> u32 {
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let r = a.wrapping_sub(b) & M;
                self.cf = a < b;
                self.of = (a ^ b) & (a ^ r) & S != 0;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $sbb(&mut self, a: u32, b: u32) -> u32 {
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let cin = self.cf as u64;
                let r = a.wrapping_sub(b).wrapping_sub(cin as u32) & M;
                self.cf = (a as u64) < b as u64 + cin;
                self.of = (a ^ b) & (a ^ r) & S != 0;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $and(&mut self, a: u32, b: u32) -> u32 {
                let r = a & b;
                self.cf = false;
                self.of = false;
                self.zs(r, 1u32 << ($bits - 1));
                r
            }
            #[inline(always)]
            pub fn $or(&mut self, a: u32, b: u32) -> u32 {
                let r = a | b;
                self.cf = false;
                self.of = false;
                self.zs(r, 1u32 << ($bits - 1));
                r
            }
            #[inline(always)]
            pub fn $xor(&mut self, a: u32, b: u32) -> u32 {
                let r = a ^ b;
                self.cf = false;
                self.of = false;
                self.zs(r, 1u32 << ($bits - 1));
                r
            }
            #[inline(always)]
            pub fn $inc(&mut self, a: u32) -> u32 {
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let r = a.wrapping_add(1) & M;
                self.of = r == S;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $dec(&mut self, a: u32) -> u32 {
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let r = a.wrapping_sub(1) & M;
                self.of = a == S;
                self.zs(r, S);
                r
            }
            /// Shifts take the count already masked to 0..31; a zero count
            /// leaves the flags alone, as on the CPU.
            #[inline(always)]
            pub fn $shl(&mut self, a: u32, n: u32) -> u32 {
                if n == 0 {
                    return a;
                }
                const M: u64 = (1u64 << $bits) - 1;
                const S: u32 = 1u32 << ($bits - 1);
                let w = (a as u64) << n;
                let r = (w & M) as u32;
                self.cf = (w >> $bits) & 1 != 0;
                self.of = ((r & S) != 0) != self.cf;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $shr(&mut self, a: u32, n: u32) -> u32 {
                if n == 0 {
                    return a;
                }
                const S: u32 = 1u32 << ($bits - 1);
                let r = ((a as u64) >> n) as u32;
                self.cf = ((a as u64) >> (n - 1)) & 1 != 0;
                self.of = a & S != 0;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $sar(&mut self, a: u32, n: u32) -> u32 {
                if n == 0 {
                    return a;
                }
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let sa = (a as $sty) as i64;
                let r = ((sa >> n) as u32) & M;
                self.cf = (sa >> (n - 1)) & 1 != 0;
                self.of = false;
                self.zs(r, S);
                r
            }
            #[inline(always)]
            pub fn $ror(&mut self, a: u32, n: u32) -> u32 {
                if n == 0 {
                    return a;
                }
                let r = $pror(a, n);
                const S: u32 = 1u32 << ($bits - 1);
                self.cf = r & S != 0;
                self.of = ((r & S) != 0) != ((r & (S >> 1)) != 0);
                r
            }
            #[inline(always)]
            pub fn $rol(&mut self, a: u32, n: u32) -> u32 {
                if n == 0 {
                    return a;
                }
                let r = $prol(a, n);
                const S: u32 = 1u32 << ($bits - 1);
                self.cf = r & 1 != 0;
                self.of = ((r & S) != 0) != self.cf;
                r
            }
            /// Two- and three-operand `IMUL`: truncated product, CF=OF=overflow.
            #[inline(always)]
            pub fn $imul(&mut self, a: u32, b: u32) -> u32 {
                const M: u32 = ((1u64 << $bits) - 1) as u32;
                const S: u32 = 1u32 << ($bits - 1);
                let p = ((a as $sty) as i64) * ((b as $sty) as i64);
                let r = (p as u32) & M;
                let ovf = ((r as $sty) as i64) != p;
                self.cf = ovf;
                self.of = ovf;
                self.zs(r, S);
                r
            }
        }

        #[inline]
        pub fn $pshl(a: u32, n: u32) -> u32 {
            (((a as u64) << n) & ((1u64 << $bits) - 1)) as u32
        }
        #[inline]
        pub fn $pshr(a: u32, n: u32) -> u32 {
            ((a as u64) >> n) as u32
        }
        #[inline]
        pub fn $psar(a: u32, n: u32) -> u32 {
            ((((a as $sty) as i64) >> n) as u32) & (((1u64 << $bits) - 1) as u32)
        }
        #[inline]
        pub fn $pror(a: u32, n: u32) -> u32 {
            let n = n % $bits;
            if n == 0 {
                return a;
            }
            ((a >> n) | (a << ($bits - n))) & (((1u64 << $bits) - 1) as u32)
        }
        #[inline]
        pub fn $prol(a: u32, n: u32) -> u32 {
            let n = n % $bits;
            if n == 0 {
                return a;
            }
            ((a << n) | (a >> ($bits - n))) & (((1u64 << $bits) - 1) as u32)
        }
    };
}

/// Write the low word / low byte / second byte of a register.  Kept out of
/// line on purpose: inlined into the lifted functions, this merge is what
/// LLVM miscompiles (see lib.rs).
#[inline(always)]
pub fn merge16(r: u32, v: u32) -> u32 {
    (opaque(r) & 0xffff_0000) | (v & 0xffff)
}
#[inline(always)]
pub fn merge8l(r: u32, v: u32) -> u32 {
    (opaque(r) & 0xffff_ff00) | (v & 0xff)
}
#[inline(always)]
pub fn merge8h(r: u32, v: u32) -> u32 {
    (opaque(r) & 0xffff_00ff) | ((v & 0xff) << 8)
}

#[cold]
#[inline(never)]
fn div_fault(b: u32) -> ! {
    if b == 0 {
        panic!("engine divided by zero")
    }
    panic!("engine DIV overflow")
}

/// Optimizer barrier: an empty `asm!` the compiler must treat as producing
/// an unknown value.  Without it LLVM miscompiles chains of partial-register
/// merges (see lib.rs); the barrier costs no instruction.
#[inline(always)]
fn opaque(r: u32) -> u32 {
    let mut r = r;
    unsafe { std::arch::asm!("/* {0} */", inout(reg) r, options(pure, nomem, nostack, preserves_flags)) };
    r
}

sized_ops!(8, f_add8, f_sub8, f_adc8, f_sbb8, f_and8, f_or8, f_xor8, f_inc8, f_dec8, f_shl8,
           f_shr8, f_sar8, f_ror8, f_rol8, f_imul8, shl8, shr8, sar8, ror8, rol8, i8);
sized_ops!(16, f_add16, f_sub16, f_adc16, f_sbb16, f_and16, f_or16, f_xor16, f_inc16, f_dec16,
           f_shl16, f_shr16, f_sar16, f_ror16, f_rol16, f_imul16, shl16, shr16, sar16, ror16,
           rol16, i16);
sized_ops!(32, f_add32, f_sub32, f_adc32, f_sbb32, f_and32, f_or32, f_xor32, f_inc32, f_dec32,
           f_shl32, f_shr32, f_sar32, f_ror32, f_rol32, f_imul32, shl32, shr32, sar32, ror32,
           rol32, i32);

impl Regs {
    // ----- multiply / divide -------------------------------------------

    /// One-operand `IMUL r/m32`: EDX:EAX = EAX * src (signed).
    #[inline(always)]
    pub fn imul1_32(&mut self, b: u32) {
        let p = (self.r[EAX] as i32 as i64) * (b as i32 as i64);
        self.r[EAX] = p as u32;
        self.r[EDX] = (p >> 32) as u32;
        let ovf = (p as i32 as i64) != p;
        self.cf = ovf;
        self.of = ovf;
    }
    /// One-operand `MUL r/m32`: EDX:EAX = EAX * src.
    #[inline(always)]
    pub fn mul1_32(&mut self, b: u32) {
        let p = self.r[EAX] as u64 * b as u64;
        self.r[EAX] = p as u32;
        self.r[EDX] = (p >> 32) as u32;
        self.cf = self.r[EDX] != 0;
        self.of = self.cf;
    }
    /// Inlined on purpose: a `&mut Regs` handed to an out-of-line function
    /// would keep every guest register in memory in the calling function.
    #[inline(always)]
    pub fn div32(&mut self, b: u32) {
        let n = ((self.r[EDX] as u64) << 32) | self.r[EAX] as u64;
        if b == 0 || self.r[EDX] >= b {
            div_fault(b);
        }
        let b = b as u64;
        self.r[EAX] = (n / b) as u32;
        self.r[EDX] = (n % b) as u32;
    }
    #[inline(always)]
    pub fn div16(&mut self, b: u32) {
        let n = ((self.r[EDX] & 0xffff) << 16) | (self.r[EAX] & 0xffff);
        assert!(b != 0, "engine divided by zero");
        let q = n / b;
        assert!(q <= 0xffff, "engine DIV overflow");
        self.r[EAX] = (self.r[EAX] & 0xffff_0000) | q;
        self.r[EDX] = (self.r[EDX] & 0xffff_0000) | (n % b);
    }
    #[inline(always)]
    pub fn div8(&mut self, b: u32) {
        let n = self.r[EAX] & 0xffff;
        assert!(b != 0, "engine divided by zero");
        let q = n / b;
        assert!(q <= 0xff, "engine DIV overflow");
        self.r[EAX] = (self.r[EAX] & 0xffff_0000) | ((n % b) << 8) | q;
    }
    #[inline(always)]
    pub fn idiv32(&mut self, b: u32) {
        let n = (((self.r[EDX] as u64) << 32) | self.r[EAX] as u64) as i64;
        let d = b as i32 as i64;
        assert!(d != 0, "engine divided by zero");
        let q = n / d;
        assert!(q >= i32::MIN as i64 && q <= i32::MAX as i64, "engine IDIV overflow");
        self.r[EAX] = q as u32;
        self.r[EDX] = (n % d) as u32;
    }
    #[inline(always)]
    pub fn idiv16(&mut self, b: u32) {
        let n = ((((self.r[EDX] & 0xffff) << 16) | (self.r[EAX] & 0xffff)) as i32) as i64;
        let d = b as i16 as i64;
        assert!(d != 0, "engine divided by zero");
        let q = n / d;
        self.r[EAX] = (self.r[EAX] & 0xffff_0000) | (q as u32 & 0xffff);
        self.r[EDX] = (self.r[EDX] & 0xffff_0000) | ((n % d) as u32 & 0xffff);
    }
    #[inline(always)]
    pub fn idiv8(&mut self, b: u32) {
        let n = (self.r[EAX] & 0xffff) as u16 as i16 as i64;
        let d = b as i8 as i64;
        assert!(d != 0, "engine divided by zero");
        let q = n / d;
        let rem = n % d;
        self.r[EAX] = (self.r[EAX] & 0xffff_0000) | ((rem as u32 & 0xff) << 8) | (q as u32 & 0xff);
    }

    // ----- string instructions -----------------------------------------

    /// `MOVSB/W/D` with or without `REP`.  Overlapping forward copies must
    /// replicate data element by element (LZ copies rely on it), so only a
    /// provably disjoint forward copy takes the bulk path.
    ///
    /// The string operations take and return the registers by value: a
    /// `&mut` to the lifted function's local `Regs` would keep it in memory.
    #[inline(never)]
    pub fn movs(mut self, c: &mut Cpu, n: u32, rep: bool) -> Regs {
        let count = if rep { self.r[ECX] } else { 1 };
        if count == 0 {
            return self;
        }
        let (src, dst) = (self.r[ESI], self.r[EDI]);
        let len = count as u64 * n as u64;
        if !self.df && (dst as u64 + len <= src as u64 || src as u64 + len <= dst as u64) {
            let s = c.chk(src, len as usize);
            let d = c.chk(dst, len as usize);
            c.mem.copy_within(s..s + len as usize, d);
            self.r[ESI] = src.wrapping_add(len as u32);
            self.r[EDI] = dst.wrapping_add(len as u32);
        } else {
            let step = if self.df { n.wrapping_neg() } else { n };
            for _ in 0..count {
                let (s, d) = (self.r[ESI], self.r[EDI]);
                match n {
                    1 => {
                        let v = c.rd8(s);
                        c.wr8(d, v)
                    }
                    2 => {
                        let v = c.rd16(s);
                        c.wr16(d, v)
                    }
                    _ => {
                        let v = c.rd32(s);
                        c.wr32(d, v)
                    }
                }
                self.r[ESI] = s.wrapping_add(step);
                self.r[EDI] = d.wrapping_add(step);
            }
        }
        if rep {
            self.r[ECX] = 0;
        }
        self
    }

    #[inline(never)]
    pub fn stos(mut self, c: &mut Cpu, n: u32, rep: bool) -> Regs {
        let count = if rep { self.r[ECX] } else { 1 };
        let step = if self.df { n.wrapping_neg() } else { n };
        for _ in 0..count {
            let d = self.r[EDI];
            match n {
                1 => c.wr8(d, self.r[EAX]),
                2 => c.wr16(d, self.r[EAX]),
                _ => c.wr32(d, self.r[EAX]),
            }
            self.r[EDI] = d.wrapping_add(step);
        }
        if rep {
            self.r[ECX] = 0;
        }
        self
    }

    #[inline(never)]
    pub fn repne_scasb(mut self, c: &Cpu) -> Regs {
        let al = self.r[EAX] & 0xff;
        let step = if self.df { u32::MAX } else { 1 };
        while self.r[ECX] != 0 {
            let v = c.rd8(self.r[EDI]);
            self.r[EDI] = self.r[EDI].wrapping_add(step);
            self.r[ECX] -= 1;
            self.f_sub8(al, v);
            if self.zf {
                break;
            }
        }
        self
    }
}

impl Cpu {
    // ----- Win32 imports (stdcall) ---------------------------------------

    pub fn arg(&self, i: u32) -> u32 {
        self.rd32(self.r[ESP].wrapping_add(4 + 4 * i))
    }

    pub fn imp_GetTickCount(&mut self) {
        // Wall-clock milliseconds: the engine throttles its in-file progress
        // reports (display event 4) on this.  Nothing else reads it, so the
        // archives do not depend on it.
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let t0 = START.get_or_init(std::time::Instant::now);
        self.ticks = t0.elapsed().as_millis() as u32;
        self.r[EAX] = self.ticks;
        self.r[ECX] = 0;
        self.r[EDX] = 0;
        self.ret(0);
    }

    /// `GlobalMemoryStatus`.  The engine sizes its I/O buffers from the free
    /// physical memory, capped by its own limits; a 32-bit process on any
    /// current machine is told 2 GB, which is what is reported here.
    pub fn imp_GlobalMemoryStatus(&mut self) {
        let p = self.arg(0);
        let st: [u32; 8] = [32, 82, 0x7FFF_FFFF, 0x7FFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF,
                            0x7FFE_0000, 0x7D1F_E000];
        for (i, v) in st.iter().enumerate() {
            self.wr32(p + 4 * i as u32, *v);
        }
        self.r[ECX] = 0;
        self.r[EDX] = 0;
        self.ret(4);
    }

    pub fn imp_VirtualAlloc(&mut self) {
        let size = self.arg(1);
        let base = self.alloc(size);
        if std::env::var_os("UHARC_TRACE").is_some() {
            eprintln!("VirtualAlloc({:#x}, {size:#x}) = {base:#x}", self.arg(0));
        }
        self.r[EAX] = base;
        self.r[ECX] = 0;
        self.r[EDX] = 0;
        self.ret(16);
    }

    pub fn imp_VirtualFree(&mut self) {
        let base = self.arg(0);
        if std::env::var_os("UHARC_TRACE").is_some() {
            eprintln!("VirtualFree({base:#x}, {:#x}, {:#x})", self.arg(1), self.arg(2));
        }
        if let Some(size) = self.blocks.remove(&base) {
            self.free_blocks.push((base, size));
        }
        self.r[EAX] = 1;
        self.r[ECX] = 0;
        self.r[EDX] = 0;
        self.ret(12);
    }

    /// Hand out a zeroed region, 64 KB aligned like `VirtualAlloc`; released
    /// blocks are reused first-fit so repeated engine runs stay bounded.
    pub fn alloc(&mut self, size: u32) -> u32 {
        let size = ((size as u64 + 0xFFFF) & !0xFFFF) as u32;
        if let Some(i) = self.free_blocks.iter().position(|&(_, s)| s >= size) {
            let (base, s) = self.free_blocks.remove(i);
            if s > size {
                self.free_blocks.push((base + size, s - size));
            }
            let b = base as usize;
            self.mem[b..b + size as usize].fill(0);
            self.blocks.insert(base, size);
            return base;
        }
        let base = self.heap_next;
        let end = base as u64 + size as u64;
        assert!(end < 0xF000_0000, "engine heap exhausted");
        self.heap_next = end as u32;
        if self.mem.len() < end as usize {
            self.mem.resize(end as usize, 0);
        }
        self.blocks.insert(base, size);
        base
    }

    // ----- C runtime (Watcom register convention) ---------------------------
    //
    // Arguments arrive in EAX, EDX, EBX, ECX and then on the stack (removed
    // by the callee); the result is returned in EAX and every other register
    // is preserved.

    pub fn cstr(&self, a: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut p = a;
        loop {
            let b = self.rd8(p) as u8;
            if b == 0 {
                return out;
            }
            out.push(b);
            p += 1;
        }
    }

    pub fn put_cstr(&mut self, a: u32, s: &[u8]) {
        self.write_bytes(a, s);
        self.wr8(a + s.len() as u32, 0);
    }

    fn file(&mut self, fp: u32) -> Option<&mut std::fs::File> {
        let i = (fp.wrapping_sub(FILE_BASE) / 16) as usize;
        self.files.get_mut(i).and_then(|f| f.as_mut())
    }

    /// `fopen(name, mode)`; only the read-only binary mode is ever used.
    pub fn host_fopen(&mut self) {
        let name = self.cstr(self.r[EAX]);
        let path = String::from_utf8_lossy(&name).into_owned();
        self.r[EAX] = match std::fs::File::open(&path) {
            Ok(f) => {
                self.files.push(Some(f));
                FILE_BASE + 16 * (self.files.len() as u32 - 1)
            }
            Err(_) => 0,
        };
        self.ret(0);
    }

    /// `fread(buf, size, n, fp)`.
    pub fn host_fread(&mut self) {
        use std::io::Read;
        let (buf, size, n, fp) = (self.r[EAX], self.r[EDX], self.r[EBX], self.r[ECX]);
        let want = (size as u64 * n as u64) as usize;
        let i = self.chk(buf, want);
        let mut got = 0usize;
        if let Some(f) = self.files.get_mut(((fp.wrapping_sub(FILE_BASE)) / 16) as usize).and_then(|f| f.as_mut()) {
            while got < want {
                match f.read(&mut self.mem[i + got..i + want]) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => got += k,
                }
            }
        }
        self.r[EAX] = if size == 0 { 0 } else { (got / size as usize) as u32 };
        self.ret(0);
    }

    /// `fseek(fp, offset, whence)`.
    pub fn host_fseek(&mut self) {
        use std::io::{Seek, SeekFrom};
        let (fp, off, whence) = (self.r[EAX], self.r[EDX] as i32 as i64, self.r[EBX]);
        let pos = match whence {
            0 => SeekFrom::Start(off as u64),
            1 => SeekFrom::Current(off),
            _ => SeekFrom::End(off),
        };
        let ok = self.file(fp).map(|f| f.seek(pos).is_ok()).unwrap_or(false);
        self.r[EAX] = if ok { 0 } else { u32::MAX };
        self.ret(0);
    }

    /// `ftell(fp)`.
    pub fn host_ftell(&mut self) {
        use std::io::Seek;
        let fp = self.r[EAX];
        self.r[EAX] = self.file(fp).and_then(|f| f.stream_position().ok()).map(|p| p as u32).unwrap_or(u32::MAX);
        self.ret(0);
    }

    /// `fwrite(buf, size, n, fp)`: the extractor collects the bytes.
    pub fn host_fwrite(&mut self) {
        let (buf, size, n) = (self.r[EAX], self.r[EDX], self.r[EBX]);
        let len = (size as u64 * n as u64) as usize;
        let i = self.chk(buf, len);
        let chunk = self.mem[i..i + len].to_vec();
        self.fwrite_sink.extend_from_slice(&chunk);
        self.r[EAX] = n;
        self.ret(0);
    }

    /// Call a lifted routine from inside a host callback: every register
    /// and flag of the interrupted engine code is preserved.
    pub fn call_nested(&mut self, entry: fn(&mut Cpu), eax: u32, edx: u32, ebx: u32, ecx: u32) -> u32 {
        let (r, f) = (self.r, (self.cf, self.zf, self.sf, self.of, self.df));
        self.r[EAX] = eax;
        self.r[EDX] = edx;
        self.r[EBX] = ebx;
        self.r[ECX] = ecx;
        self.push(EXIT_ADDR);
        entry(self);
        let out = self.r[EAX];
        self.r = r;
        (self.cf, self.zf, self.sf, self.of, self.df) = f;
        out
    }

    /// `fclose(fp)`.
    pub fn host_fclose(&mut self) {
        let i = (self.r[EAX].wrapping_sub(FILE_BASE) / 16) as usize;
        if let Some(slot) = self.files.get_mut(i) {
            *slot = None;
        }
        self.r[EAX] = 0;
        self.ret(0);
    }

    /// `memset(dst, value, count)`.
    pub fn host_memset(&mut self) {
        let (dst, v, n) = (self.r[EAX], self.r[EDX] as u8, self.r[EBX] as usize);
        let i = self.chk(dst, n);
        self.mem[i..i + n].fill(v);
        self.ret(0);
    }

    /// `_splitpath(path, drive, dir, fname, ext)`; the fifth argument is on the stack.
    pub fn host_splitpath(&mut self) {
        let path = self.cstr(self.r[EAX]);
        let ext_out = self.arg(0);
        let (drive, dir, fname, ext) = split_path(&path);
        for (out, part) in [(self.r[EDX], drive), (self.r[EBX], dir), (self.r[ECX], fname), (ext_out, ext)] {
            if out != 0 {
                self.put_cstr(out, &part);
            }
        }
        self.ret(4);
    }

    /// `_makepath(path, drive, dir, fname, ext)`; the fifth argument is on the stack.
    pub fn host_makepath(&mut self) {
        let get = |c: &Cpu, a: u32| if a == 0 { Vec::new() } else { c.cstr(a) };
        let drive = get(self, self.r[EDX]);
        let dir = get(self, self.r[EBX]);
        let fname = get(self, self.r[ECX]);
        let ext = get(self, self.arg(0));
        let path = make_path(&drive, &dir, &fname, &ext);
        let out = self.r[EAX];
        self.put_cstr(out, &path);
        self.ret(4);
    }

    /// Debug aid (lifter `LIFT_PROBES`): write the registers and the image
    /// to `$UHARC_PROBE_DIR/probe_<addr>_<n>.bin` in dbg.py's layout.
    #[allow(dead_code)]
    pub fn probe(&mut self, at: u32) {
        if std::env::var_os("UHARC_PROBE_REGS").is_some() {
            let r = &self.r;
            println!(
                "R {at:x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} ret {:x}",
                r[EAX], r[ECX], r[EDX], r[EBX], r[ESI], r[EDI], r[EBP], self.rd32(r[ESP])
            );
            if std::env::var_os("UHARC_PROBE_STACKHASH").is_some() {
                let mut h: u64 = 0xcbf29ce484222325;
                for &b in &self.mem[0x3E0000..0x3FF000] {
                    h = (h ^ b as u64).wrapping_mul(0x100000001b3);
                }
                println!("H {at:x} {h:016x}");
            }
            if let Ok(m) = std::env::var("UHARC_PROBE_MEM") {
                let (o, n) = m.split_once(':').unwrap();
                let (o, n) = (u32::from_str_radix(o, 16).unwrap(), u32::from_str_radix(n, 16).unwrap());
                let b = self.read_bytes(r[ESP] + o, n as usize);
                println!("M {at:x} {}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
            }
            return;
        }
        if let Some(dir) = std::env::var_os("UHARC_PROBE_DIR") {
            self.probes += 1;
            let mut img = Vec::with_capacity(40 + 0x8A000);
            for v in [self.r[EAX], self.r[ECX], self.r[EDX], self.r[EBX], self.r[ESP], self.r[EBP], self.r[ESI], self.r[EDI], at, 0] {
                img.extend_from_slice(&v.to_le_bytes());
            }
            img.extend_from_slice(&self.mem[0x400000..IMAGE_END as usize]);
            let p = std::path::Path::new(&dir).join(format!("probe_{at:x}_{}.bin", self.probes));
            let _ = std::fs::write(p, img);
            if let Ok(spec) = std::env::var("UHARC_PROBE_PTRS") {
                let v: Vec<u32> = spec.split(':').map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
                let mut out = Vec::new();
                for i in 0..v[1] {
                    let p = self.rd32(v[0] + 4 * i);
                    for k in 0..v[2] {
                        out.push(if p == 0 { 0 } else { self.mem.get((p + k) as usize).copied().unwrap_or(0) });
                    }
                }
                let q = std::path::Path::new(&dir).join(format!("probe_{at:x}_{}_ptrs.bin", self.probes));
                let _ = std::fs::write(q, out);
            }
        }
    }

    // ----- front-end callbacks (register convention) ----------------------

    /// Indirect calls that do not land in lifted code.
    pub fn host_call(&mut self, t: u32) {
        match t {
            HOOK_NOP => {}
            CB_PROGRESS => {
                let (ev, a, b) = (self.r[EAX] & 0xff, self.r[EDX], self.r[EBX]);
                let abort = match self.progress_hook.take() {
                    Some(mut hook) => {
                        let r = hook(self, ev, a, b);
                        self.progress_hook = Some(hook);
                        r
                    }
                    None => false,
                };
                self.r[EAX] = (self.r[EAX] & 0xffff_ff00) | abort as u32;
            }
            CB_WRITE_FILES => {
                let (buf, n) = (self.r[EAX], self.r[EDX]);
                let mut hook = self.write_hook.take().expect("no file writer installed");
                let done = hook(self, buf, n);
                self.write_hook = Some(hook);
                self.r[EAX] = done;
            }
            CB_READ_TABLE | CB_READ_DATA | CB_READ_ARCHIVE => {
                // EAX = buffer, EDX = wanted; returns the count in EAX.
                let (buf, want) = (self.r[EAX], self.r[EDX] as usize);
                let n = want.min(self.input.len() - self.in_pos);
                let src = self.in_pos;
                let i = self.chk(buf, n);
                self.mem[i..i + n].copy_from_slice(&self.input[src..src + n]);
                self.in_pos += n;
                self.r[EAX] = n as u32;
            }
            CB_WRITE | CB_WRITE_TABLE => {
                // EAX = buffer, EDX = count.
                let (buf, n) = (self.r[EAX], self.r[EDX] as usize);
                let i = self.chk(buf, n);
                let chunk = self.mem[i..i + n].to_vec();
                self.output.extend_from_slice(&chunk);
                self.r[EAX] = n as u32;
            }
            _ => panic!("engine called unknown address {t:#x}"),
        }
        self.ret(0);
    }
}

/// Watcom `_splitpath`: drive (`X:`), directory (through the last slash),
/// file name, and extension (from the last dot, dot included).
pub fn split_path(path: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut rest = path;
    let mut drive = Vec::new();
    if rest.len() >= 2 && rest[1] == b':' {
        drive = rest[..2].to_vec();
        rest = &rest[2..];
    }
    let cut = rest.iter().rposition(|&b| b == b'\\' || b == b'/').map(|i| i + 1).unwrap_or(0);
    let dir = rest[..cut].to_vec();
    let name = &rest[cut..];
    let dot = name.iter().rposition(|&b| b == b'.').unwrap_or(name.len());
    (drive, dir, name[..dot].to_vec(), name[dot..].to_vec())
}

/// Watcom `_makepath`.
pub fn make_path(drive: &[u8], dir: &[u8], fname: &[u8], ext: &[u8]) -> Vec<u8> {
    let mut p = Vec::new();
    if !drive.is_empty() {
        p.push(drive[0]);
        p.push(b':');
    }
    if !dir.is_empty() {
        p.extend_from_slice(dir);
        if !matches!(dir[dir.len() - 1], b'\\' | b'/') {
            p.push(b'\\');
        }
    }
    p.extend_from_slice(fname);
    if !ext.is_empty() {
        if ext[0] != b'.' {
            p.push(b'.');
        }
        p.extend_from_slice(ext);
    }
    p
}
