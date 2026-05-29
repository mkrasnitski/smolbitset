[![crates.io version]][crates.io link] [![ci-badge][]][ci] [![docs-badge][]][docs] [![rust-version-badge]][rust-version-link] [![license badge]][license]

# smolbitset

A crate for dynamically sized bitsets with memory usage optimizations.

Supports 64 and 32 bit targets and integrates with `serde` and `typesize`. Also supports
`no_std` environments by disabling the `std` feature. The `no_std` environment must support
`alloc`.

## Example

```rust
use smolbitset::SmolBitSet;

let mut sbs = SmolBitSet::empty();

sbs |= 1u32 << 5;
sbs >>= 5u8;
assert_eq!(sbs, SmolBitSet::from(1u64));

sbs |= !1u64;
assert_eq!(sbs, SmolBitSet::from(u64::MAX));

sbs <<= 64u16;
assert_eq!(sbs, SmolBitSet::from_bits(&(64..128).collect::<Box<[_]>>()))
```

## Const support

Constructing a `SmolBitSet` in a `const` context is supported in the following ways:
1. If the value has multiple set bits, call `SmolBitSet::new_inline`.
2. If the value has only a single set bit (i.e. it represents a flag), `SmolBitSet::flag` is
   recommended.

## Memory usage

Bitsets of small enough size are stored inline using a single `usize`, and otherwise are
allocated on the heap. See the [crate documentation][docs] for details.

| Target Pointer Size | `size_of::<SmolBitSet>` | Inline Capacity | Max Heap Capacity |
|--------------------:|------------------------:|----------------:|------------------:|
| 32 bits             | 4 bytes                 | 30 bits         | 2^36 bits         |
| 64 bits             | 8 bytes                 | 62 bits         | 2^68 bits         |

Furthermore, `SmolBitSet` has a niche optimization so `Option<SmolBitSet>` has the same size
as `SmolBitSet`.

## Limitations

* `SmolBitSet` does not implement `Copy`.
* Implementing `core::ops::Not` is also not possible (or rather complex). Related alternative
  methods are provided via `SmolBitSet::and_not` and `SmolBitSet::and_not_assign`.

## Minimum Supported Rust Version

Currently this crate supports an MSRV of Rust 1.89.0, and increasing the MSRV is considered a
breaking change.

[ci]: https://github.com/serenity-rs/smolbitset/actions
[ci-badge]: https://img.shields.io/github/actions/workflow/status/serenity-rs/smolbitset/ci.yml?branch=main&style=flat-square
[docs]: https://docs.rs/smolbitset
[docs-badge]: https://img.shields.io/docsrs/smolbitset/latest?style=flat-square
[crates.io link]: https://crates.io/crates/smolbitset
[crates.io version]: https://img.shields.io/crates/v/smolbitset.svg?style=flat-square
[rust-version-badge]: https://img.shields.io/badge/rust-1.89.0+-93450a.svg?style=flat-square
[rust-version-link]: https://blog.rust-lang.org/2025/08/07/Rust-1.89.0/
[license]: LICENSE
[license badge]: https://img.shields.io/crates/l/poise.svg?style=flat-square&color=yellow
