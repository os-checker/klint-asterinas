# Stack frame size checks

`klint` can check if there're functions with large stack frames.
Checks are performed by diassembling all functions and look for `subq $imm, %rsp` instruction.

This check is disabled by default.
It can be enabled by adding `#![warn(klint::stack_frame_too_large)]` on crate root (or, equivalently, enable the lint with CLI flags).

The stack frame size limit can be configured by `--cfg=CONFIG_FRAME_WARN="<limit>"`.
If you're building the kernel, this is cfg option is automatically passed by `KBUILD`.

## Limitations

Currently only x86-64 is supported.
The lint is silently ignored on other architectures.
