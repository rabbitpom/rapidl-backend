#!/bin/bash

cd rust_app || exit
rustup override set nightly || exit
cargo lambda build --compiler cargo --release --target x86_64-unknown-linux-gnu --jobs 3 || exit
cd ..
