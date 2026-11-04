# Horizon: EELisp all the way down (a RISC-V operating system)

*Status: long-horizon idea — 2026-11-04. Builds on [self-hosting.md](self-hosting.md) and
[eedb.md](eedb.md). Nothing here is needed for the earlier plans. It's the direction they open up.*

## Why this isn't crazy

Lisp has run directly on hardware before: the Lisp Machines (Symbolics Genera, LMI) in the
1980s, and later hobby and research systems such as Mezzano (Common Lisp), Loko Scheme and
Movitz. The ideas are proven. What's new here is a combination: a small, well-specified language
(EELisp after the self-hosting work), a clean open ISA (RISC-V), and a database-native storage
layer (eedb).

EELisp's design already fits this. The core does no I/O and uses no threads or platform types,
and the host decides what the language can reach. On bare metal, the "host" is a few hundred
lines of trap handling and device drivers.

## Two paths, in order

### Path A — EELisp as the userland on a tiny native kernel (near-term, a demo)

Build the current Rust kernel as `no_std` + `alloc` for `riscv64gc-unknown-none-elf` and boot it
under **QEMU's `virt` machine**:

- A minimal boot stub: set up the stack, the heap and the trap vector, a UART console, and a timer.
- The EELisp REPL on the serial console is the "shell".
- A block device (virtio-blk) exposed through the same device interface eedb uses. The database
  is the filesystem.

Things to check first (not verified): `serde_json` works without `std` given `alloc`. `regex`
needs `std` by default, so bare metal might need `regex-automata` or a tiny EELisp regex. And
`dates` needs a clock from the timer.

**Done when:** QEMU boots to an EELisp prompt, `(deftable …)` / `(insert …)` persist across
reboots on the virtio disk.

This is a weekend-to-weeks project once the kernel is small (self-hosting Phase 1). It still
depends on Rust, but EELisp is everything the user touches.

### Path B — Self-hosted: EELisp compiling itself to RISC-V (the real thing)

This is the "no single host defines EELisp" goal taken to the end: no host language at all.
It's a ladder:

1. **A low-level subset ("Pre-EELisp").** A restricted EELisp with no GC allocation, fixed-size
   integers, explicit memory (`peek`/`poke` on addresses, building on the existing `bytes`
   module), and no closures that escape. It's the language for the parts that can't depend on a
   runtime: the allocator, the GC, trap handlers, drivers. Prior art: **PreScheme**, the
   restricted Scheme that Scheme48's virtual machine is written in.
2. **A compiler written in EELisp** that emits RISC-V machine code (RV64GC is a small, regular
   target). Stage 0 runs it on today's Rust interpreter. Stage 1 is the compiler compiling
   itself. Stage 2 recompiles with stage 1, and its output must match stage 1 byte for byte.
   That's the classic self-hosting proof.
3. **The runtime in Pre-EELisp:** allocator, garbage collector (start with a simple copying or
   mark-sweep collector), value tagging, tail calls, error values. It's compiled by step 2.
4. **A boot image:** a tiny assembly entry stub, then compiled Pre-EELisp (memory, traps,
   drivers), then compiled EELisp (reader, eval, prelude, libraries, REPL).
5. **The system layer in EELisp:** scheduler (cooperative first; EELisp has no threads, which
   keeps this honest), drivers (UART, timer, virtio-blk, virtio-net), and eedb on the block
   device as the system's storage.

**Done when:** the boot image builds on a machine with no Rust (by running the compiler on the
previous boot image, or on any other EELisp kernel), boots in QEMU, and the EELisp test suite
passes inside it.

## How the earlier plans feed this

| Earlier work | What it becomes here |
|---|---|
| Phase 0: metacircular eval + diff harness | the conformance oracle for the native compiler |
| Phase 1: kernel / library split | the exact list of what the runtime must provide |
| Phase 5: compiler | the RISC-V back end is just one more target |
| eedb device interface | the block-device driver API; the database **is** the filesystem |
| eedb change feeds | system events (files changed, records added) without polling |
| Host-installed callbacks | system calls: the capability model the OS exposes to programs |
| Docs / self-documenting | `(source fn)` works on the kernel itself, Lisp-Machine style |

The last row deserves a note. A system where every function, including the drivers, carries its
own documentation and source, and where you can inspect and redefine it from the REPL, is the
part of the Lisp Machine experience most worth having again.

## Design ideas worth keeping in mind now

- **Database as filesystem.** It's been tried (BeOS's BFS had queryable attributes, and
  Microsoft's WinFS never shipped). Records with types and queries instead of byte streams in
  paths, with a file-like view for compatibility. eedb makes this natural.
- **Capabilities, not ambient authority.** EELisp already works this way: a script can reach
  only what the host installed. An OS built on that gives each program the callbacks it is
  granted and nothing else. This is good security by construction.
- **One address space, safe language.** Lisp Machine–style single address space, with the
  language providing isolation instead of the MMU. It's simpler, but every program must be
  EELisp (or verified). An MMU-based process model can come later.
- **Cooperative first.** No threads in the core means cooperative scheduling is the honest
  starting point. Preemption can come from timer interrupts plus safe points in the compiler.

## Hardware

Start and stay in **QEMU `virt`** (riscv64) for a long time: it's deterministic, scriptable and
debuggable with gdb, and it has virtio devices. Real boards come later (various RISC-V SBCs
exist; pick one when Path A works, and check its documentation for UART, timer and storage
access).

## What would kill it

Not technical impossibility; Lisp operating systems have been built. Scope is the real risk.
Each rung of the ladder must be useful by itself:

- The metacircular eval is a better test suite.
- The compiler makes EELisp faster everywhere, not only on bare metal.
- eedb is useful in browsers and sandboxes.
- Path A is a fun demo and a portability test for the kernel.

If the OS never happens, nothing built on the way is wasted. That's the test for doing any of it.

## Open questions

- Is the Pre-EELisp subset a separate dialect, or EELisp plus declarations (`(declare (no-alloc))`)?
- Compiler IR: compile straight to RISC-V, or go through a small portable IR (which also enables
  WASM and other targets)?
- GC: precise (needs compiler stack maps) or conservative to start?
- How much POSIX compatibility, if any? (Possibly none: a REPL-first, database-first system that
  doesn't pretend to be Unix.)
