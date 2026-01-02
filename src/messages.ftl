klint_monomorphize_encountered_error_while_instantiating =
    the above error was encountered while instantiating `{$kind} {$instance}`

klint_monomorphize_encountered_error_while_instantiating_global_asm =
    the above error was encountered while instantiating `global_asm`

klint_monomorphize_recursion_limit =
    reached the recursion limit while instantiating `{$instance}`
    .note = `{$def_path_str}` defined here

klint_build_error_referenced_without_symbol =
    found a reference to `build_error` in the object file, but no associated symbol is found

klint_build_error_referenced_without_instance =
    symbol `{$symbol}` references `build_error` in the object file, but no associated instance is found

klint_build_error_referenced_without_debug =
    `{$kind} {$instance}` contains reference to `build_error`
    .note = attempt to reconstruct line information from DWARF failed: {$err}

klint_build_error_referenced =
    this `build_error` reference is not optimized away

klint_stack_frame_limit_help =
    set stack size limit with `--cfg CONFIG_FRAME_WARN="<size-in-bytes>"`

klint_stack_frame_limit_missing =
    stack size limit is not set, default to {$default} bytes

klint_stack_frame_limit_invalid =
    stack size limit is set to `{$setting}` bytes, which cannot be parsed as integer

klint_stack_frame_too_large =
    stack size of `{$instance}` is {$stack_size} bytes, exceeds the {$frame_limit}-byte limit
    .note = the stack size is inferred from instruction `{$insn}` at {$section}+{$offset}

klint_duplicate_diagnostic_item_in_crate =
    duplicate klint diagnostic item in crate `{$crate_name}`: `{$name}`
    .note = the diagnostic item is first defined in crate `{$orig_crate_name}`

klint_diagnostic_item_first_defined =
    the klint diagnostic item is first defined here
