# Utilify

Utilify is a set of macOS menu-bar utilities. It shows network activity and
storage, keeps the Mac awake, controls the fans and reads temperatures, and
has a data workbench that ports [CyberChef](https://github.com/gchq/CyberChef)'s
operations to native Rust.

## The menu-bar icon

The icon redraws every tick (0.5, 1, 2 or 5 s). It shows the current
download ▼ and upload ▲ rates (`9.2MB/s`, fixed width so the item doesn't
jitter), a bar graph of the last 24 download samples, and a storage bar under
it. In the storage bar, the translucent track is the disk's capacity and the
solid fill is used space.

It can also show a temperature from any sensor (CPU Core Average by default)
and the fastest fan's speed, as `80°C` over `5775rpm`, next to the rates or
instead of them. Turn these on in Settings or from the menu's "Show
Temperature/Fan Speed in Menu Bar".

The icon is an SVG rendered with resvg and shown as a template image, so it
follows the light or dark menu bar.

Left click opens the dashboard. Right click opens the menu: Open Dashboard,
Open Workbench, Keep Awake, Fans, the refresh interval, the "Show Speeds /
Temperature / Fan Speed in Menu Bar" toggles, and Quit. Hovering the item
shows the current rates, free space and any keep-awake session.

## The dashboard

The dashboard has six tabs:

- Overview: the 2-minute rate graph, current rates, storage and keep awake.
- Network: link capacity and use, session totals, and an on-demand speed test
  that measures latency idle and under load.
- Storage: used space over the last 30 days.
- Awake: keep-awake sessions.
- Fans: fan control and temperature sensors.
- Settings: what the menu bar shows, storage colours, notifications, the
  health checks, Reset and Quit.

It follows the system's light or dark appearance. ⌘1 to ⌘6 pick a tab,
⌃Tab and ⌃⇧Tab step through them, ⌘, opens Settings, ⌘B opens the workbench,
Esc or ⌘W closes the dashboard and ⌘Q quits. Icon-only buttons have tooltips
that show their shortcut.

### Storage colours

The storage bar is green below 50% used, yellow from 50% and red from 80%.
Both lines can be changed in Settings. Crossing into red posts a
notification, once per crossing. It re-arms after usage drops 2 points below
the red line, and starting up already in the red doesn't notify.

### Keep awake

Keep awake has the options from
[Amphetamine](https://apps.apple.com/app/amphetamine/id937984704)'s menu. You'll
find them in the menu's Keep Awake submenu and on the Awake tab.

You can start a session:

- indefinitely (⌘I)
- for 5 to 45 minutes or 1 to 12 hours, or a custom length or "until 18:30"
  from the Awake tab
- while an app runs (pick one of the running apps; the session ends when it
  quits)
- while a file downloads: any `.download`, `.crdownload` or `.part` file in
  Downloads (⌘F). It ends a minute after the last one goes.
- while the network is busy. It ends after 2 minutes under 50 KB/s.

End Current Session is ⌘X. The Awake tab shows the time remaining and whether
the session started by hand or at launch.

Session options let the display sleep, let the Mac sleep when the lid
closes, and allow the screen saver after N minutes without input. Blocking
lid-close sleep only works on AC power, because macOS ignores it on battery.
Blocking display sleep also blocks the screen saver, so Utilify starts the
screen saver itself.

Quick settings start a session at launch, notify when a session ends, and
launch Utilify at login (macOS 13 or later, bundled app only).

While a session runs, a pill shows left of the menu-bar icon. The total time
kept awake is stored across launches in `awake.json`. Sessions hold IOKit
power assertions, which macOS drops if Utilify quits or crashes. Check them
with `pmset -g assertions`.

### Fans

Fan control works like [Macs Fan Control](https://crystalidea.com/macs-fan-control),
on the Fans tab and in the menu's Fans submenu.

Each fan is listed ("Left side" and "Right side" on two-fan Macs, otherwise
"Fan N") with its current, minimum and maximum speed and how it's driven:

- Auto: the system's own control.
- Constant: a speed you set, in 100 rpm steps.
- Sensor: pick a sensor and two temperatures. The fan stays at its minimum up
  to the first, reaches its maximum at the second, and moves linearly in
  between.

A curve only lowers the fan once the sensor is 2 °C below the reading that
set the speed, and changes speed by at most 1000 rpm/s up and 250 rpm/s down,
so it doesn't hunt. If a curve's sensor disappears, that fan goes back to
automatic.

There are two built-in presets, Automatic (every fan on automatic) and Full
blast (every fan at its maximum). Save the current controls under a name to
make your own. Rename one with the pencil, or delete it (the toast offers
Undo). Editing a
fan inside a preset edits the preset. Otherwise the edit becomes the "Custom"
set. The menu lists the fans with their control, the presets with a check on
the active one, and Show Fans…. Presets are kept in
`<Application Support>/dev.utilify.app/fans.json`.

The tab also lists every temperature sensor by group (CPU, GPU, SoC, battery,
storage and so on), refreshed every 2 s. It flags the readings as stale if
they stop arriving.

Setting speeds needs the fan helper (see Install). Without it, or for a
non-admin user, the tab shows a banner saying what to do, with the commands
ready to copy, and the controls stay disabled. Speeds and temperatures still
show. The tab also warns when another tool, such as Macs Fan Control, holds
the fans, when fans run high on battery, and when the helper reports an
error. Quitting Utilify or choosing Automatic puts every fan back on
automatic. If Utilify crashes, the helper does it when the connection drops.
On Intel Macs the fans are read-only.

### Notifications

Every notification has its own switch in Settings › Notifications, under a
master switch. Clicking one opens the tab it's about. Utilify can tell you
when:

- the fans are held at full speed on battery. It waits 10 minutes, then
  reminds you hourly while it lasts, with Set to Automatic and Snooze 1 Hour.
- the battery runs low during a keep-awake session, at 20% (adjustable) and
  again at half that, capped at 10%. It offers End Session.
- the Mac switches to battery while a session blocks lid-close sleep, which
  macOS ignores on battery.
- a timed session will end in 5 minutes, with Add 30 Minutes. Off by
  default.
- macOS is throttling the Mac to cool it down.
- the overheating guard takes the fans back.
- the fan helper stops answering while you have the fans set.
- another app takes the fans.
- the network has been offline for a minute. Off by default.
- a speed test finishes while the dashboard is closed.
- storage crosses into red.
- a keep-awake session ends.

Each condition has to last a short while before it posts, and posts once per
occurrence (hourly for fans on battery). Its notification is taken back when
the condition clears, and quitting takes back the rest. The rules live in
`utilify-core/src/alerts.rs`. Notifications only work from the bundled app,
because macOS needs a bundle identity to deliver them.

### Health checks

Shortly after launch and every 15 minutes, Utilify checks that:

- its data folder is writable and its saved files parse
- the disk and the network can be read
- `networkQuality` is there
- the fan helper answers and fan readings keep coming

The results are in Settings › Health, which also has a Check now button. A
failing check puts a banner on Overview and a dot on the Settings tab.

When a save fails, a banner says why and Utilify retries every 30 s. When a
saved file doesn't parse, Utilify moves it aside as
`<name>.corrupt-<time>.json` instead of overwriting it, and says so once.

## The workbench

Open it with ⌘B in the menu or the chef's hat in the dashboard header. It's a
full window in CyberChef's layout: operations on the left (search,
favourites, CyberChef's categories), the recipe in the middle, and input over
output on the right.

Click or drag operations into the recipe. Each step can be disabled, given a
breakpoint (Step runs on from it), removed, or dragged into place. When a step
fails, the output shows which one, with buttons to show it or disable it.

Auto bake reruns the recipe on a background thread after every change, and
⌘↩ bakes by hand. Input is typed text or a file: use Open file (⌘O) or drop
one on the input. Output can be copied, saved, or used as the next input.
⌘F jumps to the operation search.

Paste takes a recipe as CyberChef JSON, its `Op_Name(args)` chef format, or a
CyberChef URL (with its input). Copy gives chef format. The recipe, the input
(up to 256 KB), auto bake and favourites are kept in
`<Application Support>/dev.utilify.app/workbench.json`.

So far CyberChef's "Data format" category is ported, and the list shows the
rest as "soon". The codecs come from maintained libraries and the
utilify-codecs crates (see `crates/utilify-chef/DEPENDENCIES.md`). While the
window is open, Utilify shows in the Dock.

## The command line

`utilifyctl` reads and changes what the dashboard and the menu do, from a
terminal. It talks to the running app, so a change made with it shows up in
the menu bar and the dashboard straight away, and a change made in the
dashboard shows up in `utilifyctl` on its next refresh. The workbench's
operations aren't in it yet; `utilifyctl workbench open` opens the window.

```sh
utilifyctl                       # the overview
utilifyctl watch                 # a live view of the dashboard's tabs
utilifyctl network -w            # one tab, live
utilifyctl speedtest             # runs one and waits; Ctrl-C cancels it
utilifyctl speedtest copy        # the last result, on the clipboard
utilifyctl storage               # use, bar colours, 30 days of history
utilifyctl awake start 2h        # or: until 18:30, app Xcode, download, network
utilifyctl awake extend 30m
utilifyctl awake stop
utilifyctl fans use full-blast   # or automatic, custom, or one of your presets
utilifyctl fans set left 3000    # or auto, max, curve cpu_core_average 50 85
utilifyctl fans save Quiet       # also rename and delete
utilifyctl sensors
utilifyctl settings              # every setting, with its key
utilifyctl settings set awake_allow_display_sleep off
utilifyctl health run            # health fix <id> runs a check's fix
utilifyctl open fans             # opens the dashboard on a tab
utilifyctl workbench open
```

`utilifyctl --help` lists the rest: problems, snooze, login, launch and quit.
Each command also has its own `--help`.

The live view (`watch`, or `-w` on a tab) switches tabs with 1 to 6 or the
arrow keys and quits with q. It redraws at the app's refresh interval. Each
frame is written to the screen in one go, and nothing is cleared first, so it
doesn't flicker. Charts are Unicode block characters, and each cell shows
eighths of its height. The live view keeps one connection open and asks for
one snapshot per refresh, so it barely registers in Activity Monitor.

`--json` prints any command's data or outcome as JSON, for scripts (the
live view has no JSON form). The exit status is 0
on success, 1 when something is refused (with the reason on stderr), and 2
when Utilify isn't running. `utilifyctl launch` starts it. The settings
commands also work while Utilify is closed: they edit `settings.json`, which
the app reads when it starts.

Settings take the same values the dashboard offers. On/off settings take `on`
or `off`. The refresh interval is one of 0.5 s, 1 s, 2 s or 5 s. Yellow
starts below red, and both stay between 5% and 95%. A temperature sensor can
be given by id or by name. Fan speeds are clamped to the fan's range, and
curves run between 20 and 110 °C. Fan changes need the fan helper, as they do
in the dashboard.

The app listens on `<Application Support>/dev.utilify.app/cli.sock`. Only
your user can open it: the file is mode 0600 and the app checks the uid of
each connection. If that path is too long for a socket, both sides use the
per-user temporary folder instead. `install.sh` links
`Utilify.app/Contents/MacOS/utilifyctl` into the first writable folder of
`/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin`. Change the folder
with `UTILIFY_CLI_DIR`, or skip the link with `UTILIFY_NO_CLI=1`.

## Install

```sh
./install.sh
```

Builds the current source and installs it, so re-run it after pulling or
editing. It needs Rust (https://rustup.rs) and a checkout of
[utilify-codecs](https://github.com/imran-vz/utilify-codecs) next to this
one (`../utilify-codecs`), which the workbench uses by path. The script installs
`cargo-packager` if it's missing, builds `Utilify.app`, puts
it in `/Applications` (or `~/Applications` if that isn't writable; override
with `UTILIFY_INSTALL_DIR`), and launches it (skip that with
`UTILIFY_NO_LAUNCH=1`).

On Apple Silicon it also installs the fan helper, which lets Utilify set
fan speeds. Reading fans and temperatures works for any user, but writing to
the SMC (the chip that runs the fans) needs root, so `utilify-fanhelper` runs
as a root LaunchDaemon: `/Library/PrivilegedHelperTools/dev.utilify.fanhelper`,
loaded by `/Library/LaunchDaemons/dev.utilify.fanhelper.plist`, logging to
`/var/log/utilify-fanhelper.log`. The script asks for your password once
(`sudo`) to put it there. The helper only accepts admin users (its socket,
`/var/run/dev.utilify.fanhelper.sock`, is `root:admin` 0660), only changes
fan modes and target speeds (`Ftst`, `F{n}Md`, `F{n}Tg`), clamps speeds to
each fan's range, and returns the fans to automatic when Utilify disconnects
or goes 15 s without a request, when it's stopped, and after a crash. Skip it
with `UTILIFY_NO_FAN_HELPER=1`; remove it with
`scripts/uninstall-fan-helper.sh`, which stops it (fans back to automatic)
and deletes those files. Intel Macs skip it: fans are read-only there.

## Build and run

```sh
cargo run -p utilify-app            # debug, unbundled
cargo run -p utilify-cli -- status  # utilifyctl against the running app
cargo packager --release -p utilify-app --formats app   # Utilify.app (LSUIElement, no Dock icon)
```

Debug builds read a few environment variables:

- `UTILIFY_DEV_OPEN_WORKBENCH=1` opens the workbench at launch.
- `UTILIFY_DEV_OPEN_DASHBOARD=1` opens the dashboard at launch. With it,
  `UTILIFY_DEV_TAB=network` (or `storage`, `awake`, `fans`, `settings`) picks
  the tab, `UTILIFY_DEV_SPEED_TEST=1` starts a speed test (with its section unfolded),
  `UTILIFY_DEV_AWAKE=1` starts a 30-minute keep-awake session, and
  `UTILIFY_DEV_PIN=1` keeps it open when focus moves away.
- `UTILIFY_DEV_APPEARANCE=light` (or `dark`) overrides the system appearance,
  for checking both palettes.
- `UTILIFY_DEV_SCREEN=main` opens the dashboard on the main display, whatever
  screen the pointer is on. Handy for Retina screenshots.
- `UTILIFY_DEV_QUIT_AFTER=10` quits through the normal path after 10 s.

To try fan control without touching the real fans, run the helper with its
in-memory SMC and point a debug build at it:

```sh
cargo run -p utilify-fanhelper -- --fake --no-root-check --socket /tmp/fans.sock &
UTILIFY_DEV_FAN_SOCKET=/tmp/fans.sock UTILIFY_DEV_OPEN_DASHBOARD=1 UTILIFY_DEV_TAB=fans cargo run -p utilify-app
```

`cargo test -p utilify-core real_helper -- --ignored` (after building
`utilify-fanhelper`) runs the fan controller against that fake helper. `cargo run -p utilify-core --example render_icon -- /tmp/out 8`
samples for 8 s and writes the menu-bar icon as PNGs (`UTILIFY_AWAKE=1` adds
the keep-awake pill, `UTILIFY_FANS=1` the temperature and fan readout).

The logo, an extruded "U" with a green level block, is generated by
`python3 scripts/brand/gen.py`. It writes the app icon source
(`assets/icons/app/utilify.svg`) and the bare mark for dark and light
backgrounds (`assets/brand/`, which the dashboard header uses). After changing
it, run `cargo run -p utilify-core --example render_app_icon` to regenerate
`utilify.png` and `utilify.icns`, which the packager bundles, and
`python3 scripts/brand/cards.py` to redraw the site's share image and the
social images.

## Layout

It uses the same stack as [Guidy](https://github.com/imran-vz/Guidy): a Rust workspace, GPUI through
`gpui-kit = "=0.7.3"`, `tray-icon` for the status item, objc2 for AppKit, and
Guidy's Zed One theme. The dashboard uses Geist / Geist Mono; the menu-bar
text uses Lilex.

```
crates/
├─ utilify-core/   No UI and no gpui dependency.
│    net.rs       sysctl(NET_RT_IFLIST2): 64-bit byte counters + link speed per
│                 interface; NetMeter turns readings into rates. Only physical
│                 `enN` ports are counted (VPN `utun` traffic also crosses them).
│    disk.rs      startup volume: Finder's "important usage" capacity, statvfs fallback
│    disk_history.rs  used space every 15 min, kept 30 days
│                 (<Application Support>/dev.utilify.app/disk_history.json)
│    awake.rs     keep-awake sessions, IOKit power assertions, idle time, and the
│                 awake total (<Application Support>/dev.utilify.app/awake.json)
│    speedtest.rs runs macOS's `networkQuality -c` (macOS 12+), with a watchdog
│    sensors.rs   named temperature sensors (HID + SMC), Apple Silicon
│    fans.rs      fan controls, curves with hysteresis, presets
│                 (<Application Support>/dev.utilify.app/fans.json)
│    fans/        client.rs (the helper's socket), controller.rs (one tick:
│                 targets, only-changed SETs, PINGs, AUTO ALL), service.rs
│                 (the 2 s background thread)
│    power.rs     AC or battery, charge, Low Power Mode (IOKit power sources)
│    alerts.rs    the notification rules: debounce, repeats, snooze, retract
│    health.rs    the periodic self-checks
│    monitor.rs   one tick: rates, rolling history, session totals, cached disk
│    icon.rs      menu-bar SVG + resvg rasterizer
│    ipc.rs       utilifyctl's socket: requests, snapshots, client and server
│    format.rs, history.rs, settings.rs (<Application Support>/dev.utilify.app/settings.json)
├─ utilify-smc/    The SMC: keys, typed values, fans, temperatures (IOKit, no root
│                 needed to read), and the fan helper's wire protocol.
│                 `cargo run -p utilify-smc --example fans` prints fans and all T* keys.
├─ utilify-fanhelper/  binary `utilify-fanhelper`, the root LaunchDaemon that sets
│                 fan speeds. `--socket <path> --no-root-check` (and `--fake` for an
│                 in-memory SMC) run it as a normal user for testing.
├─ utilify-chef/   The workbench engine. No UI and no gpui dependency.
│    dish.rs      the value between operations, with CyberChef's text/bytes rules
│    args.rs      argument kinds (CyberChef's), values and typed access
│    recipe.rs    steps, baking, breakpoints; CyberChef JSON, chef format, URLs
│    ops/         the operations, one module per CyberChef category
│    catalog.rs   CyberChef's full category list, ported or not
│    session.rs   the workbench state kept across launches
│    tests/cyberchef_vectors.rs  CyberChef's own operation tests, run on the port
├─ utilify-cli/    binary `utilifyctl`. No gpui dependency.
│    main.rs      the commands, and the live view's loop
│    view.rs      what each command shows, one renderer per dashboard tab
│    chart.rs     sparklines, column charts and bars from block characters
│    term.rs      colour, size, the live view's screen and keys (libc, no
│                 terminal library)
│    parse.rs     durations, times, modes, fans, controls and presets
└─ utilify-app/    binary `utilify`
     lib.rs       startup (Accessory policy), sampler timer, tray event loop
     control.rs   answers utilifyctl on the main thread, through the same
                  functions as the menu and the dashboard
     tray.rs      status item, menu, icon updates (main thread only)
     dashboard.rs the GPUI popover: the six tabs, shortcuts, toasts, and
                  opening and closing the window
     dashboard/awake.rs  the Awake tab and Overview's keep-awake control
     dashboard/fans.rs   the Fans tab: banners, presets, fans, sensors
     dashboard/status.rs problem banners, the Notifications and Health sections
     system.rs    running apps (NSWorkspace) and launch at login (SMAppService)
     ui.rs        the design system: light and dark colours, the type and
                  corner scales, and the parts both windows are built from
     placement.rs dashboard position and height under the clicked status item
     workbench.rs the workbench window: state, baking, persistence, header
     workbench/   operations list, recipe steps, input/output, widgets
```

### Porting CyberChef operations

Names, arguments and defaults match CyberChef, so its recipes load unchanged,
but each operation follows its format's specification rather than CyberChef:
where CyberChef deviates from the spec, the port doesn't copy it.
`crates/utilify-chef/tests/vectors/cyberchef.json` holds CyberChef's operation
tests (exported by `scripts/cyberchef/export.mjs`); every vector whose
operations are all ported must pass, apart from the places CyberChef deviates from
the spec, listed in `known_differences.txt`. `VECTORS=Base64 cargo test -p utilify-chef --test
cyberchef_vectors -- --nocapture` runs a subset. For behaviour the vectors
don't cover, `scripts/cyberchef/oracle.mjs` bakes recipes with real CyberChef;
see `scripts/cyberchef/README.md`. Ported operations follow CyberChef
(Apache-2.0, Crown Copyright). `crates/utilify-chef/PORTING.md` has the
port's status and the rule for dependencies: maintained, well-used libraries
for encodings, formats and crypto, with our code as the CyberChef adapter.

### Notes

- Link capacity is the driver-reported `ifi_baudrate`. For Ethernet that's
  the negotiated speed. For Wi-Fi it's the current PHY rate, which moves with
  signal quality. Usage is the busier direction divided by capacity, since
  links are full duplex.
- The "primary" interface is the active `enN` port that has carried the most
  traffic since boot.
