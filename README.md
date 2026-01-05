# PicoDSP

**PicoDSP** is a Minimoog-inspired virtual analog emulator running on the **Raspberry Pi Pico 2 (RP2350)**. 
It leverages the power of the [infinitedsp-core](https://crates.io/crates/infinitedsp-core) library to deliver a rich, low-latency audio experience directly from a microcontroller.

This project serves as a flagship implementation of `infinitedsp-core` in an embedded, `no_std` environment using the [Embassy](https://embassy.dev/) async framework.

## Features

*   **Virtual Analog Engine:** A Minimoog-inspired architecture with 3 antialiased Oscillators + Noise, Mixer, ZDF Ladder Filter, and Envelopes.
*   **Effects Chain:** Built-in Delay (Stereo), Reverb (Mono), and Stereo Widener.
*   **USB Audio Class 1.0:** Acts as a USB Microphone, streaming synthesized audio directly to your PC/Mac/Linux machine at 48kHz, 16-bit stereo. No DAC required for recording!
*   **USB MIDI:** Full MIDI control over parameters (Cutoff, Resonance, Envelopes) and Note input.
*   **Dual Core Processing:**
    *   **Core 0:** Handles USB communication (Audio/MIDI/CDC) and system tasks.
    *   **Core 1:** Dedicated DSP processing loop for maximum stability and low latency.
*   **Preset Management:** Save and load presets to the Pico's internal Flash memory via MIDI SysEx.

## Hardware Requirements

*   **Raspberry Pi Pico 2** (RP2350)
*   Micro-USB cable

*Note: No external DAC (Digital-to-Analog Converter) is strictly required as the audio is streamed via USB. However, there are plans to extend the code to support I2S DACs and DIN MIDI in the future.*

## Getting Started

### Prerequisites

*   [Rust](https://www.rust-lang.org/tools/install) (stable)
*   `thumbv8m.main-none-eabihf` target:
    ```bash
    rustup target add thumbv8m.main-none-eabihf
    ```
*   [picotool](https://github.com/raspberrypi/picotool) 

### Building

```bash
cargo build --release
```

### Flashing

1.  Hold the **BOOTSEL** button on your Pico 2 while plugging it in.
2.  Run:
    ```bash
    cargo run --release
    ```
    *(This uses `picotool` configured in `.cargo/config.toml` to load the ELF directly)*

Or, if you prefer UF2:
```bash
picotool uf2 convert -t elf target/thumbv8m.main-none-eabihf/release/picodsp picodsp.uf2 --family rp2350-arm-s
# Then drag picodsp.uf2 to the RPI-RP2 drive
```

## Usage

1.  Connect the PicoDSP to your computer via USB.
2.  It will appear as:
    *   **Audio Device:** "PicoDSP (infinitedsp ...)" (Input device)
    *   **MIDI Device:** "PicoDSP MIDI"
3.  Open your DAW or standalone synth host.
4.  Select "PicoDSP" as your **Audio Input** to hear the synth.
5.  Route MIDI to "PicoDSP MIDI" to play notes.

### MIDI CC Map

| CC # | Parameter |
|------|-----------|
| 1    | Mod Wheel |
| 5    | Portamento Time |
| 64   | Sustain Pedal |
| 71   | Filter Resonance |
| 74   | Filter Cutoff |
| 120  | All Sound Off |
| 123  | All Notes Off |

## Architecture

The project is structured as follows:

*   `src/common`: Shared constants and data structures.
*   `src/control`: MIDI handling and parameter logic.
*   `src/data`: Preset definitions and Flash storage management.
*   `src/dsp`: DSP graph construction (Oscillators, Filters, Effects).
*   `src/tasks`: The main tasks for Core 0 (System/USB) and Core 1 (Audio).
*   `src/usb`: USB descriptors and device implementation.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
