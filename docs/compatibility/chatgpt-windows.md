# ChatGPT Classic for Windows compatibility report

Status: dependency discovery for F3-WN-021

Snapshot date: 2026-08-07, America/Mexico_City

Store product: `9NT1R1C2HH7J` (`ChatGPT Classic`)

Implementation target: first usable window on Android ARM64, without package or credential redistribution

## Result

The pinned 1.46 MB executable is **not the ChatGPT application payload**. It is a product-specific, Microsoft-signed copy of the generic managed `StoreInstaller.exe`. It must contact Microsoft services and ask the Windows package manager to acquire and register an MSIX/AppX package graph. Translating this bootstrapper does not constitute running ChatGPT.

The current Microsoft Store catalog advertises separate `x64` and `arm64` payloads under package family `OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0`. The catalog does not publish their concrete package version through its public package-manifest response; it reports `Unknown`. The payload itself was not acquired during this audit because `winget download` required interactive Microsoft Entra authentication. Consequently, this report does **not** claim that the payload uses WinUI, WebView2, Electron, or any other UI runtime. Those questions become manifest/file-level gates below.

This is the compatibility conclusion:

- The Store bootstrapper is the wrong executable for Prisma's x86-64 DBT gate. It is a PE32 CLR host requiring .NET Framework plus Store, COM and package-deployment services.
- The `x64` MSIX payload is the correct DBT target after legitimate user acquisition.
- The `arm64` MSIX payload is useful as a control: it can bypass x86 translation on an ARM64 host, but it still requires Prisma's Windows application model and Win32 compatibility layer.
- F3-WN-022 remains blocked until the user-acquired package graph and its exact manifests are captured outside Git, package identity is implemented, and its observed runtime dependencies are satisfied.

## Evidence rules

The report uses these labels deliberately:

- **Verified/local:** derived directly from the pinned artifact on Danny's machine.
- **Verified/catalog:** returned by Microsoft Store or documented by OpenAI/Microsoft.
- **Inference:** a compatibility risk suggested by product behavior, not proof of a concrete dependency.
- **Unknown:** requires the actual MSIX payload, runtime traces or both.

## Pinned bootstrapper

| Field | Observed value | Evidence |
|---|---|---|
| Path | `C:\Users\daedg\Downloads\Prisma-Windows-Targets\ChatGPT-Classic-official-installer.exe` | Verified/local |
| Official source | `https://get.microsoft.com/installer/download/9NT1R1C2HH7J?cid=website_cta_psi` | `targets.lock.json`; endpoint returns `ChatGPT Classic Installer.exe` |
| Size | 1,462,848 bytes | Verified/local |
| SHA-256 | `acfdd2b7af6a0f97086a1d16630e82da2561ab3148f0f1b706c4165bdb3a0097` | Verified/local |
| File version | `22607.722.4.0` | Verified/local PE version resource |
| Product version | `22607.0722.04.0+d51b52927c5be7ff1d7943161faf05c493099514` | Verified/local PE version resource |
| Original filename | `StoreInstaller.exe` | Verified/local PE version resource |
| PE/CLR format | PE32, `IMAGE_FILE_MACHINE_I386`, MSIL assembly `StoreInstaller` | Verified/local PE and CLR metadata |
| Bootstrap UI/runtime | WPF on `.NETFramework,Version=v4.7.2` | Verified/local assembly target and references (`PresentationFramework`, `PresentationCore`, `System.Xaml`) |
| Native import | `mscoree.dll!_CorExeMain` | Verified/local import directory |
| Subsystem | Windows GUI | Verified/local optional header |
| Authenticode | Valid; signer `Microsoft Corporation` | Verified/local with `Get-AuthenticodeSignature` |
| Signer thumbprint | `C4F5DC349876887A6B082DD85BFFB091969488EC` | Verified/local; certificate is short lived, so the timestamped signature and pinned artifact hash are the durable evidence |

The executable contains no MSIX payload and has no appended overlay after its `WIN_CERTIFICATE`. Its executable prefix was byte-identical to a Store installer generated for another product in a controlled comparison; only the certificate table differed. This supports the inference that Microsoft binds the generic installer to the selected Store product through its per-download signed credential rather than a plainly embedded package. Prisma must preserve the entire signed executable if testing bootstrap behavior; renaming is harmless, modifying any byte is not.

## Store package facts

The [OpenAI Windows app article](https://help.openai.com/en/articles/9982051-using-the-chatgpt-windows-app) identifies Microsoft Store product `9NT1R1C2HH7J`, documents installation with `winget`, and advertises Windows 10 on x64 and ARM64. The current [Microsoft Store listing](https://apps.microsoft.com/detail/9NT1R1C2HH7J) calls that product ChatGPT Classic. OpenAI separately explains the distinction between the previous app and the new unified desktop app in [Moving to the new ChatGPT desktop app](https://help.openai.com/en/articles/20001276-moving-to-the-new-chatgpt-desktop-app).

The official Store catalog endpoints returned the following on the snapshot date:

| Field | Current catalog value | Classification |
|---|---|---|
| Product ID | `9NT1R1C2HH7J` | Verified/catalog |
| Title | `ChatGPT Classic` | Verified/catalog |
| Publisher | OpenAI | Verified/catalog |
| Package family | `OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0` | Verified/catalog |
| Architectures | `x64`, `arm64` | Verified/catalog |
| Installer type/scope | `msstore`, per user | Verified/catalog |
| Concrete package version | `Unknown` | Verified/catalog; must be obtained from the acquired package identity |
| Approximate download size | 297,806,137 bytes | Verified/catalog; mutable metadata, not a package hash |
| Maximum installed size | 368,394,240 bytes | Verified/catalog; mutable metadata |
| Store minimum OS | Windows 10 build 18362 | Verified/catalog |
| OpenAI help minimum OS | Windows 10 build 17763 | Verified/documentation |
| Declared capabilities | `runFullTrust`, `webcam`, `microphone`, `internetClientServer`, `privateNetworkClientServer` | Verified/catalog |
| App extension | `com.microsoft.windows.copilotkeyprovider` | Verified/catalog |

The Store metadata is stricter than the OpenAI help article about the minimum Windows build. Prisma must use build 18362 as the current compatibility baseline until the acquired `AppxManifest.xml` proves a different `TargetDeviceFamily MinVersion`.

The catalog responses are available directly from Microsoft:

- [Product metadata](https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/9NT1R1C2HH7J?market=MX&locale=en-US&deviceFamily=Windows.Desktop)
- [Package manifest metadata](https://storeedgefd.dsx.mp.microsoft.com/v9.0/packageManifests/9NT1R1C2HH7J?Market=MX)

These endpoints describe catalog and installer selection; they do not replace the signed package manifest or prove its file-level dependencies.

## Package and bootstrap behavior

### Verified

1. `StoreInstaller.exe` is a .NET Framework 4.7.2 WPF bootstrapper, not a self-extracting ChatGPT installer. Its own UI is WPF, not evidence of the ChatGPT payload's UI framework.
2. Its code references Microsoft Store catalog, entitlement and package-deployment APIs, including package-manager operations such as `AddPackageByUriAsync`.
3. The Store publishes architecture-specific `x64` and `arm64` installers with the same package family name.
4. Store metadata marks the package per-user and full trust.
5. `winget show --id 9NT1R1C2HH7J --source msstore` reports that offline distribution is supported, but `winget download` on this machine still requested interactive Entra authentication before returning package material.

### Required Prisma behavior

Prisma must not emulate the Microsoft Store merely to launch a package the user has already obtained. The practical path is:

1. The user or an authorized administrator acquires the exact Store package and dependency packages under Microsoft's terms.
2. A host-side importer verifies signatures and copies package files into a private Prisma prefix without committing them to Git.
3. Prisma registers a minimal package identity and package graph sufficient for manifest activation, URI protocol activation, local state and dependency resolution.
4. The loader launches the manifest's real executable with the package environment, not `StoreInstaller.exe`.
5. Updates are explicit new fixtures with new hashes; no test silently tracks `latest`.

Microsoft documents that Store-packaged applications may rely on automatically installed framework packages and that architecture-specific bundle selection happens during deployment. See [App package formats](https://learn.microsoft.com/en-us/windows/msix/package/app-package-formats) and [Package and deploy Windows apps](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/).

## UI framework: WinUI and WebView2

### What is verified

Nothing in the pinned bootstrapper or public catalog proves the UI framework of the ChatGPT payload. The name `OpenAI.ChatGPT-Desktop`, a web-like product experience, Store packaging, or a full-trust capability is not sufficient evidence for WinUI 3, WinUI 2/UWP, WPF, WebView2, Electron or CEF.

### Payload inspection gate

After legitimate acquisition, record the package version, architecture, package hash and every `<PackageDependency>` from `AppxManifest.xml`, then inspect its files without executing them. At minimum search for:

- `Microsoft.UI.Xaml.dll`, `Microsoft.WindowsAppRuntime.*` and `Microsoft.WindowsAppSDK.*` for Windows App SDK/WinUI 3;
- framework identities such as `Microsoft.WindowsAppRuntime`, `Microsoft.VCLibs` or `Microsoft.NET.Native` in the package graph;
- `WebView2Loader.dll`, `msedgewebview2.exe` or WebView2 COM identifiers;
- `chrome_elf.dll`, `resources.pak`, `icudtl.dat` and Electron/Chromium metadata;
- CEF libraries and subprocess executables;
- executable import tables, CLR metadata and child-process command lines.

Only observed files, manifests or traces may flip a dependency from **Unknown** to **Verified**.

### Conditional compatibility work

- **If WinUI 3/Windows App SDK is observed:** implement framework-package dependency resolution, VCLibs, Windows App Runtime initialization, XAML window activation and the required COM/WinRT surface. Microsoft documents the framework, Main and Singleton package model in [Windows App SDK deployment for packaged apps](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/deploy-packaged-apps).
- **If WebView2 is observed:** provide a matching ARM64 or translated x64 WebView2 Runtime, COM loader interfaces, subprocess creation, job objects, shared-memory transport, profile storage, sandbox policy and GPU/compositor surfaces. Microsoft documents Evergreen and Fixed Version runtime models in [Introduction to WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/).
- **If Electron/CEF is observed:** do not install WebView2 speculatively. Treat the bundled Chromium runtime, sandbox, crash handler, locales, ICU data and GPU subprocesses as the concrete graph.

## Networking and TLS

### Verified

- The Store declares `internetClientServer` and `privateNetworkClientServer`.
- Product features require online conversations, search, uploads and account access.
- No API hostname, TLS implementation, proxy behavior, WebSocket use or certificate-store dependency was established from the bootstrapper.

### Gates

1. Capture DNS names and process ownership from a clean, user-authorized native run; never record bearer tokens, cookies or message content.
2. Determine whether traffic uses WinHTTP, WinINet, Schannel, WebView2 or a bundled Chromium/BoringSSL stack.
3. Validate DNS, IPv4/IPv6, SNI, ALPN, HTTP/2, WebSocket/SSE, system clock, trusted roots and revocation behavior.
4. Validate system proxy, explicit proxy and proxy-auth failure paths. PAC support is a separate gate.
5. Keep Android trust material out of the guest by default; expose a deliberate Windows-compatible certificate store and deterministic teardown for network handles, sockets and background processes.

## Authentication and protected state

Authentication details are **Unknown** until a native trace identifies the activation and storage mechanisms. Likely risks include a system-browser OAuth handoff, custom URI activation, loopback callbacks, Web Account Manager, cookies, Credential Locker and DPAPI-protected state. Those are inferences, not current facts.

The acceptance policy is fixed:

- A human controls sign-in, MFA, device approval and logout.
- Automated tests do not type, persist, export or replay credentials, cookies, refresh tokens or account identifiers.
- Deep-link and protocol activation must route to the correct package instance.
- Guest secrets remain inside the app's private prefix and are removed by an explicit reset/delete operation.
- A logged-out launch and first usable window must pass before any authenticated test is attempted.

## Graphics, input, capture and audio

The catalog confirms webcam and microphone capabilities, while the official product description advertises screenshots, Advanced Voice and a global `Alt+Space` companion window. It does not identify the underlying APIs.

The first-window gate must trace and support the APIs actually used for:

- top-level window creation, activation, resize, DPI and multi-window behavior;
- Direct3D/DXGI, DirectComposition or software rendering;
- DirectWrite/fonts, clipboard, keyboard, pointer, IME and accessibility;
- file picker and drag/drop for uploads;
- global hotkey registration and companion-window focus;
- screen capture permission and texture transfer;
- MMDevice/WASAPI capture/playback and media codecs if voice is enabled;
- camera capture only after the basic text-chat path is stable.

Prisma must close child processes, GPU resources, audio clients, capture sessions and package-local files explicitly during app shutdown and prefix restart. A window that appears while a renderer or broker leaks across restart is not a pass.

## Redistribution and test-fixture policy

ChatGPT Classic is proprietary and Store-delivered. Neither the generic installer nor a downloaded MSIX, dependency package, license, account state or credential may be committed to this repository or redistributed with Prisma. `Offline Distribution Supported: true` is a Store deployment capability, not a grant for Prisma to republish the package.

Tests must accept a user-owned path, verify an explicitly approved hash, and skip with a precise message when the fixture is absent. Installation and use remain subject to [OpenAI Terms of Use](https://openai.com/policies/terms-of-use), [OpenAI Privacy Policy](https://openai.com/policies/privacy-policy), and [Microsoft Store license terms](https://aka.ms/microsoft-store-license).

## Blockers and implementation gates

| Gate | Evidence required | Current state |
|---|---|---|
| C0: authorized payload | User-acquired x64 MSIX/bundle plus dependencies, outside Git | Blocked: bootstrapper only |
| C1: immutable fixture | Package identity, version, architecture, size, SHA-256 and valid signature | Blocked: catalog version is `Unknown` |
| C2: package graph | Parsed manifests, dependencies, capabilities, extensions and executable identity | Blocked: payload unavailable |
| C3: activation | Package identity registered; real x64 entry process starts through Prisma | Blocked: AppModel/WinRT surface incomplete |
| C4: UI stack | Observed WinUI/WebView2/Electron/CEF graph and child processes start | Unknown until C2 |
| C5: first window | Stable logged-out window, text/input/resize/close, no crash | Not attempted |
| C6: network/TLS | Native-equivalent TLS and streaming connection without secret capture | Not attempted |
| C7: user auth | Manual sign-in/deep-link works; logout and state reset verified | Not attempted |
| C8: basic chat | User sends one benign prompt and receives a streamed response | Not attempted |
| C9: clean restart | All processes, sockets, GPU/audio objects and files close; second launch isolated | Not attempted |

The dependency order is C0 → C1 → C2 → C3 → C4 → C5. Network and authentication work starts only after the unauthenticated window is stable. Advanced Voice, webcam, screenshot capture and companion hotkeys are follow-up gates; they must not delay proving basic text-chat compatibility.

## Reproduction

Run the repository's read-only analyzer from the repository root:

```powershell
& '.\tools\windows-apps\chatgpt\analyze-installer.ps1'
```

Run it without network catalog queries:

```powershell
& '.\tools\windows-apps\chatgpt\analyze-installer.ps1' -Offline
```

Independent local checks:

```powershell
$artifact = 'C:\Users\daedg\Downloads\Prisma-Windows-Targets\ChatGPT-Classic-official-installer.exe'
Get-Item -LiteralPath $artifact | Select-Object Length, VersionInfo
Get-FileHash -LiteralPath $artifact -Algorithm SHA256
Get-AuthenticodeSignature -LiteralPath $artifact | Format-List
& 'C:\Program Files\7-Zip\7z.exe' l -slt -- $artifact
```

Query the official Store selection manifest without downloading or installing:

```powershell
$uri = 'https://storeedgefd.dsx.mp.microsoft.com/v9.0/packageManifests/9NT1R1C2HH7J?Market=MX'
Invoke-RestMethod -Uri $uri | ConvertTo-Json -Depth 20
winget show --id 9NT1R1C2HH7J --source msstore --accept-source-agreements
```

After a user-authorized installation, capture identity and manifest without copying package contents into Git:

```powershell
$package = Get-AppxPackage -Name 'OpenAI.ChatGPT-Desktop'
$package | Select-Object Name, PackageFullName, PackageFamilyName, Architecture, Version, InstallLocation
$manifest = Get-AppxPackageManifest -Package $package.PackageFullName
$manifest.Package.Dependencies | Format-List
$manifest.Package.Applications.Application | Format-List
```

Do not use the locally installed `OpenAI.Codex`/new unified ChatGPT package as evidence for ChatGPT Classic. OpenAI documents them as separate applications, and their package identities and dependency graphs may differ.
