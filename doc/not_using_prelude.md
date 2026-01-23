# Not using prelude

This lint will warn on imports that exist in prelude, but a full path is used instead.
This is a style issue commonly made in the Linux kernel.

For example, the following code is non-optimal:
```rust
use kernel::error::Error;
```

because it is already available via prelude.
This should be used instead.

```rust
use kernel::prelude::*;
```

Currently, doctests are ignored by checking if the crate name contains `doctest`.
This should probably be explicitly allowed in kernel Makefile instead, when it gains direct support for `klint`.
