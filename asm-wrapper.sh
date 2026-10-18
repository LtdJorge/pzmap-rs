#!/bin/sh
# rustdoc runs this as: asm-wrapper.sh <path-to-rustc> <args...>
echo "$@" >> /tmp/doctest-rustc-args.txt
exec "$@" --emit=link,asm,llvm-ir
