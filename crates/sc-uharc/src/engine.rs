//! Safe wrapper around the lifted codec engine.
//!
//! One `Engine` corresponds to one run of the original program: its globals
//! persist from call to call exactly as they did in UHARC.EXE, where the data
//! stream and then the three table blocks were coded by the same engine
//! instance one after another.

use uharc_lifted::engine;
use uharc_lifted::x86::{self, Cpu, EAX, EBX, ECX, EDX, ESP};

pub struct Engine {
    cpu: Cpu,
}

impl Engine {
    pub fn new() -> Self {
        let cpu = Cpu::new();
        Engine { cpu }
    }

    /// Direct access to the engine's globals, for the front end's settings.
    pub fn cpu(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    /// Call a lifted routine with the Watcom register convention: arguments
    /// in EAX, EDX, EBX, ECX, then `stack` (pushed last-first, removed by the
    /// callee).  Returns EAX.
    pub fn call(&mut self, entry: fn(&mut Cpu), eax: u32, edx: u32, ebx: u32, ecx: u32, stack: &[u32]) -> u32 {
        let c = &mut self.cpu;
        c.r[ESP] = x86::STACK_TOP;
        for &v in stack.iter().rev() {
            c.push(v);
        }
        c.push(x86::EXIT_ADDR);
        c.r[EAX] = eax;
        c.r[EDX] = edx;
        c.r[EBX] = ebx;
        c.r[ECX] = ecx;
        entry(c);
        assert_eq!(c.r[ESP], x86::STACK_TOP, "engine left the stack unbalanced");
        c.r[EAX]
    }

    /// `malloc` from the original's runtime heap (the lifted `0x40def0`).
    /// The front end keeps its file lists there, as UHARC.EXE did: the
    /// engine's heap then never runs completely empty between calls.
    pub fn malloc(&mut self, size: u32) -> u32 {
        let p = self.call(engine::f_40def0, size, 0, 0, 0, &[]);
        assert!(p != 0, "out of engine heap");
        p
    }

    fn run(&mut self, entry: fn(&mut Cpu)) -> u32 {
        let c = &mut self.cpu;
        c.r[ESP] = x86::STACK_TOP;
        c.push(x86::EXIT_ADDR);
        entry(c);
        assert_eq!(c.r[ESP], x86::STACK_TOP, "engine left the stack unbalanced");
        c.r[EAX]
    }

    /// Compress the file data stream (`total` bytes) with `method`; the
    /// engine pulls it through the lifted read callback, which walks the file
    /// list the caller has laid out in engine memory.
    pub fn compress_stream(&mut self, method: u8, total: u32) -> Result<Vec<u8>, u32> {
        let c = &mut self.cpu;
        c.output.clear();
        c.r[EAX] = method as u32;
        c.r[ECX] = total;
        c.r[EDX] = x86::CB_READ_DATA;
        c.r[EBX] = x86::CB_WRITE;
        let rc = self.run(engine::f_40d560);
        if rc != 0 {
            return Err(rc);
        }
        Ok(std::mem::take(&mut self.cpu.output))
    }

    /// Decompress one block: `input` is the block as stored (already
    /// decrypted), `out_len` the decoded size.  `files` routes the output
    /// through the installed file writer instead of collecting it.
    pub fn decompress(&mut self, method: u8, input: &[u8], out_len: u32, files: bool) -> Result<Vec<u8>, u32> {
        let c = &mut self.cpu;
        c.input = input.to_vec();
        c.in_pos = 0;
        c.output.clear();
        c.r[EAX] = method as u32;
        c.r[ECX] = out_len;
        c.r[EDX] = x86::CB_READ_ARCHIVE;
        c.r[EBX] = if files { x86::CB_WRITE_FILES } else { x86::CB_WRITE_TABLE };
        let rc = self.run(engine::f_40d440);
        if rc != 0 {
            return Err(rc);
        }
        Ok(std::mem::take(&mut self.cpu.output))
    }

    /// `free()` on the runtime heap (the lifted `0x40ca70`).
    pub fn free(&mut self, p: u32) {
        self.call(engine::f_40ca70, p, 0, 0, 0, &[]);
    }

    /// Compress `input` with `method` (0..=87).  `data` selects the read
    /// callback the original used for file data rather than its tables.
    pub fn compress(&mut self, method: u8, input: &[u8], data: bool) -> Result<Vec<u8>, u32> {
        let c = &mut self.cpu;
        c.input = input.to_vec();
        c.in_pos = 0;
        c.output.clear();
        c.r[EAX] = method as u32;
        c.r[ECX] = input.len() as u32;
        c.r[EDX] = if data { x86::CB_READ_DATA } else { x86::CB_READ_TABLE };
        c.r[EBX] = x86::CB_WRITE;
        let rc = self.run(engine::f_40d560);
        if rc != 0 {
            return Err(rc);
        }
        Ok(std::mem::take(&mut self.cpu.output))
    }
}
