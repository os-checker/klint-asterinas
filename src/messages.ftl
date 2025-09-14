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
