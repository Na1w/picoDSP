#!/bin/bash

cargo build --release

if [ $? -eq 0 ]; then
    picotool uf2 convert -t elf target/thumbv8m.main-none-eabihf/release/picodsp picodsp.uf2 --family rp2350-arm-s
else
    echo "Failed to create uf2 image."
fi
