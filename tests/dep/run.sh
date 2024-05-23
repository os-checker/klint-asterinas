#!/usr/bin/env bash

cd $(dirname "${BASH_SOURCE[0]}")

RUSTFLAGS="-Zcrate-attr=feature(register_tool) -Zcrate-attr=register_tool(klint) --edition=2021"

$KLINT spin.rs --crate-type lib $RUSTFLAGS
$KLINT bin.rs --extern spin -L. $RUSTFLAGS
