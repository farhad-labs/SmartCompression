//! The codec engine of UHARC 0.6b, translated instruction by instruction from
//! the original executable (`work/lift/lift.py`), and the machine model it
//! runs on.
//!
//! Compiler note: rustc 1.98's LLVM miscompiled the inline partial-register
//! merge `(r & 0xffff0000) | (w & 0xffff)` inside these very large generated
//! functions at every optimisation level above 0 (in `f_428dd0` the upper
//! half of EAX was lost, which made the PPM decoder divide by zero).  The
//! lifter therefore emits such merges as calls to the out-of-line helpers
//! `x86::merge16/merge8l/merge8h`, which avoids the problem.
#![allow(non_snake_case)]

pub mod engine;
pub mod x86;
