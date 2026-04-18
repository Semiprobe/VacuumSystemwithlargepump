# Vacuum Control — HiPace 1200 / Duo 35

A VB.NET Windows Forms application for PC control of a vacuum system built
around a Pfeiffer HiPace 1200 turbopump and a Duo 35 rotary-vane backing
pump, with a TPG 366 MaxiGauge gauge controller.

The app runs in **demo mode** when no hardware is present — the pressure,
turbo speed, and valve states are simulated in a physically plausible way,
so you can exercise the full pumpdown / vent automation on any Windows PC
without a rig attached.

## Features

- Live readout of chamber ion gauge (IG) and Pirani (PG)
- Live readout of HiPace 1200 speed, drive current, and state
- Status indicators for V1 (roughing), V2 (foreline), V3 (vent), and Duo 35
- One-button automated **Pump down**:
  1. Open V1, rough the chamber with the Duo 35
  2. When PG ≤ crossover pressure, close V1 and open V2
  3. Start the turbopump, wait for full speed
  4. Wait for IG to reach high vacuum
- One-button automated **Vent**:
  1. Stop the turbo, wait for spindown
  2. Close V2 (isolate the foreline)
  3. Open V3 and admit N₂
  4. When chamber reaches atmosphere, close V3
- **Abort** at any point — drives all valves closed and the turbo off
- Live log with timestamps
- Automatic fallback to demo mode if the COM ports aren't reachable

## Build and run

Requires Visual Studio 2022 (17.8 or newer) with the **Visual Basic** workload,
or the .NET 8 SDK on the command line.

**Visual Studio:** open `VacuumControl.sln`, press F5. The project restores
`System.IO.Ports` from NuGet on first build.

**Command line:**

```
cd VacuumControl
dotnet build
dotnet run --project VacuumControl
```

## Project layout

```
VacuumControl/                 <- solution root
  VacuumControl.sln
  README.md
  .gitignore
  VacuumControl/               <- project folder
    VacuumControl.vbproj
    Program.vb
    MainForm.vb
    VacuumHardware.vb
    PumpSequencer.vb
```

| File | Responsibility |
|---|---|
| `Program.vb` | Entry point |
| `VacuumHardware.vb` | `IVacuumHardware` interface, `Tpg366Driver`, `Tc1200Driver`, `RealHardware`, `DemoHardware` |
| `PumpSequencer.vb` | Background state machine for pumpdown / vent |
| `MainForm.vb` | WinForms UI (built programmatically — no `.designer.vb`) |

All hardware access goes through `IVacuumHardware`. The sequencer and the
UI never see a real serial port — they only see the interface, so demo mode
and real mode are fully interchangeable.

## Protocol notes

### TPG 366 MaxiGauge (mnemonic protocol)

- 9600 baud, 8N1, no handshake
- `PR<n><CR><LF>` → `<ACK><CR><LF>` then send `<ENQ>` → `status,pressure<CR><LF>`
- Status codes: 0 = ok, 1 = underrange, 2 = overrange, 3 = sensor error,
  4 = sensor off, 5 = no sensor, 6 = identification error
- Relays 1-6 are settable via the `SPS` mnemonic (used for V1 in this app)

Protocol reference: Pfeiffer document **BG5511BEN** (TPG 366 Communication
Protocol). Download from the Pfeiffer website or check the manual PDF
included in the earlier chat search results.

### TC 1200 (Pfeiffer Vacuum Protocol)

- 9600 baud, 8N1, no handshake
- Telegram: `addr(3) action(2) param(3) len(2) data(n) cksum(3) <CR>`
- Action `00` = read, `10` = write; checksum = sum of ASCII codes mod 256
- Parameters used in this code:
  - `P:010` pumping station on/off
  - `P:012` vent valve enable
  - `P:025` backing pump relay
  - `P:309` actual rotation speed (Hz)
  - `P:310` drive current (0.01 A units)
  - `P:315` nominal speed (Hz)
  - `P:346` error code
  - `P:788` example accessory output (used here for V2 — verify against your
    accessory configuration)

> ⚠️ **Verify parameter numbers before deploying against a real TC 1200.**
> Some accessory-output parameters depend on how the TC 1200 was
> configured at the factory (or during commissioning). Open the TC 1200
> manual PDF (from the earlier chat) and confirm each parameter number
> matches your firmware revision. `RealHardware.SetValveV2` and
> `SetBackingPumpOn` are the two lines most likely to need adjustment.

### Valve wiring assumed by this code

| Valve | Driven by |
|---|---|
| V1 roughing | TPG 366 relay 1 (pressure-switched or manual via `SPS`) |
| V2 foreline | TC 1200 accessory output (P:788 placeholder) |
| V3 vent     | TC 1200 dedicated vent valve (P:012) |
| Duo 35      | TC 1200 backing-pump relay (P:025 placeholder) |

Change these in `RealHardware` if your wiring differs.

## Demo mode

If you tick **Demo mode** and click **Connect**, or if the serial ports
fail to open, the app loads `DemoHardware`. It models:

- Chamber pressure as an exponential in `log10(p)` converging to a target
  that depends on valve / pump state (atmospheric when vented, ~5×10⁻²
  mbar on roughing, ~10⁻⁸ mbar on turbo).
- Foreline pressure that rises slightly when the turbo is pumping gas
  into it.
- Turbo speed with realistic ramp-up / ramp-down time constants.

Demo mode accelerates real-world time constants so a full pumpdown runs
in under a minute — useful for demos and CI tests without waiting the
real 10-15 min.

## Safety notes for real hardware

- **Always interlock V1 closed while the turbo is at speed.** The
  sequencer does this logically, but a hardware interlock (a wired
  pressure switch from the TPG 366 or a relay on the TC 1200) is strongly
  recommended.
- **Never open V3 (vent) while the turbo is above ~10% nominal speed.**
  Fast venting at high rotation can damage the bearing. The vent sequence
  waits for spindown below the threshold set in
  `PumpSequencer.TurboSpindownThresholdPct`.
- **The TC 1200 has its own safety logic** (fore-line pressure, bearing
  temperature). This app respects its commands and reads its error code,
  but does not override its interlocks.

## Next steps worth considering

- Add a foreline Pirani on TPG 366 channel 3 and display it (one extra
  label and one extra poll call).
- Log pressure/speed to CSV for leak-up measurements.
- Add a scheduled bakeout timer (the HiPace 1200 supports this through
  the TMS parameter set).
- Plot the pumpdown curve with a simple chart control.
