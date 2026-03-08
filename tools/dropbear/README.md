Optional local overrides for Dropbear binaries used by MagiskV SSH API packaging.

Directory layout:

- `tools/dropbear/arm64-v8a/dropbear`
- `tools/dropbear/armeabi-v7a/dropbear`
- `tools/dropbear/x86_64/dropbear`
- `tools/dropbear/x86/dropbear`

Build behavior:

- By default, Gradle downloads per-ABI binaries from:
  - `https://github.com/ribbons/android-dropbear/releases/download/DROPBEAR_2025.89/`
- If local override exists (`tools/dropbear/<abi>/dropbear`), it is used instead of downloaded asset.
- Selected binary is packed into APK JNI libs as `libdropbear.so`.
- Installer/live setup renames and installs as `/data/adb/magisk/dropbear`.
- If neither downloaded nor local binary is available for an ABI, runtime falls back to system `dropbear`/`sshd`.
