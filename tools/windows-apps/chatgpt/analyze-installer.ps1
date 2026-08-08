[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$ArtifactPath = 'C:\Users\daedg\Downloads\Prisma-Windows-Targets\ChatGPT-Classic-official-installer.exe',

    [string]$ProductId = '9NT1R1C2HH7J',

    [ValidatePattern('^[A-Z]{2}$')]
    [string]$Market = 'MX',

    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$resolvedArtifact = (Resolve-Path -LiteralPath $ArtifactPath).Path
$bytes = [System.IO.File]::ReadAllBytes($resolvedArtifact)
if ($bytes.Length -lt 256) {
    throw "Artifact is too small to be a PE image: $resolvedArtifact"
}

$peOffset = [BitConverter]::ToUInt32($bytes, 0x3c)
if ($peOffset + 24 -gt $bytes.Length) {
    throw "Invalid PE header offset: $peOffset"
}

$peSignature = [Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4)
if ($peSignature -ne "PE`0`0") {
    throw "PE signature not found at offset $peOffset"
}

$machineValue = [BitConverter]::ToUInt16($bytes, $peOffset + 4)
$optionalHeaderOffset = $peOffset + 24
$optionalMagic = [BitConverter]::ToUInt16($bytes, $optionalHeaderOffset)
$dataDirectoryOffset = switch ($optionalMagic) {
    0x10b { $optionalHeaderOffset + 96 }
    0x20b { $optionalHeaderOffset + 112 }
    default { throw ('Unsupported optional-header magic: 0x{0:x}' -f $optionalMagic) }
}

$certificateDirectoryOffset = $dataDirectoryOffset + (4 * 8)
$clrDirectoryOffset = $dataDirectoryOffset + (14 * 8)
$certificateOffset = [BitConverter]::ToUInt32($bytes, $certificateDirectoryOffset)
$certificateSize = [BitConverter]::ToUInt32($bytes, $certificateDirectoryOffset + 4)
$clrRva = [BitConverter]::ToUInt32($bytes, $clrDirectoryOffset)
$clrSize = [BitConverter]::ToUInt32($bytes, $clrDirectoryOffset + 4)
$subsystem = [BitConverter]::ToUInt16($bytes, $optionalHeaderOffset + 68)

$machine = switch ($machineValue) {
    0x014c { 'i386' }
    0x8664 { 'x86_64' }
    0xaa64 { 'arm64' }
    default { 'unknown-0x{0:x4}' -f $machineValue }
}

$assembly = $null
try {
    $assemblyName = [Reflection.AssemblyName]::GetAssemblyName($resolvedArtifact)
    $loadedAssembly = [Reflection.Assembly]::LoadFile($resolvedArtifact)
    $targetFramework = $loadedAssembly.GetCustomAttributesData() |
        Where-Object { $_.AttributeType.FullName -eq 'System.Runtime.Versioning.TargetFrameworkAttribute' } |
        Select-Object -First 1
    $assembly = [ordered]@{
        name = $assemblyName.Name
        version = $assemblyName.Version.ToString()
        processor_architecture = $assemblyName.ProcessorArchitecture.ToString()
        target_framework = if ($targetFramework) {
            $targetFramework.ConstructorArguments[0].Value
        } else {
            $null
        }
        references = @($loadedAssembly.GetReferencedAssemblies() |
            Sort-Object Name |
            ForEach-Object {
                [ordered]@{
                    name = $_.Name
                    version = $_.Version.ToString()
                }
            })
    }
} catch [System.BadImageFormatException] {
    # A native PE has no managed assembly metadata.
}

$item = Get-Item -LiteralPath $resolvedArtifact
$hash = (Get-FileHash -LiteralPath $resolvedArtifact -Algorithm SHA256).Hash.ToLowerInvariant()
$signature = Get-AuthenticodeSignature -LiteralPath $resolvedArtifact

$result = [ordered]@{
    schema = 'prisma-chatgpt-installer-analysis/v1'
    artifact = [ordered]@{
        path = $resolvedArtifact
        size = $item.Length
        sha256 = $hash
        file_version = $item.VersionInfo.FileVersion
        product_version = $item.VersionInfo.ProductVersion
        company = $item.VersionInfo.CompanyName
        original_filename = $item.VersionInfo.OriginalFilename
    }
    pe = [ordered]@{
        format = if ($optionalMagic -eq 0x10b) { 'PE32' } else { 'PE32+' }
        machine = $machine
        machine_value = '0x{0:x4}' -f $machineValue
        subsystem = switch ($subsystem) {
            2 { 'Windows GUI' }
            3 { 'Windows CUI' }
            default { "unknown-$subsystem" }
        }
        clr_header_rva = '0x{0:x}' -f $clrRva
        clr_header_size = $clrSize
        certificate_file_offset = $certificateOffset
        certificate_size = $certificateSize
        managed_assembly = $assembly
    }
    authenticode = [ordered]@{
        status = $signature.Status.ToString()
        status_message = $signature.StatusMessage
        signer_subject = if ($signature.SignerCertificate) {
            $signature.SignerCertificate.Subject
        } else {
            $null
        }
        signer_thumbprint = if ($signature.SignerCertificate) {
            $signature.SignerCertificate.Thumbprint
        } else {
            $null
        }
        timestamp_subject = if ($signature.TimeStamperCertificate) {
            $signature.TimeStamperCertificate.Subject
        } else {
            $null
        }
    }
    store = $null
}

if (-not $Offline) {
    $manifestUri = "https://storeedgefd.dsx.mp.microsoft.com/v9.0/packageManifests/$ProductId`?Market=$Market"
    $productUri = "https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/$ProductId`?market=$Market&locale=en-US&deviceFamily=Windows.Desktop"

    try {
        $packageManifest = Invoke-RestMethod -Uri $manifestUri -Headers @{
            'User-Agent' = 'Prisma compatibility analyzer'
        }
        $product = Invoke-RestMethod -Uri $productUri -Headers @{
            'User-Agent' = 'Prisma compatibility analyzer'
        }

        $manifestVersion = $packageManifest.Data.Versions | Select-Object -First 1
        $payload = $product.Payload
        $result.store = [ordered]@{
            product_id = $payload.ProductId
            title = $payload.Title
            publisher = $payload.PublisherName
            package_family_names = @($payload.PackageFamilyNames)
            platforms = @($payload.Platforms)
            installers = @($manifestVersion.Installers | ForEach-Object {
                [ordered]@{
                    architecture = $_.Architecture
                    installer_type = $_.InstallerType
                    package_family_name = $_.PackageFamilyName
                    scope = $_.Scope
                    download_command_prohibited = $_.DownloadCommandProhibited
                }
            })
            manifest_package_version = $manifestVersion.PackageVersion
            approximate_size_bytes = $payload.ApproximateSizeInBytes
            maximum_install_size_bytes = $payload.MaxInstallSizeInBytes
            capabilities = @($payload.PackageAndDeviceCapabilities)
            last_update_utc = $payload.LastUpdateDateUtc
            product_manifest_uri = $productUri
            package_manifest_uri = $manifestUri
        }
    } catch {
        $result.store = [ordered]@{
            product_id = $ProductId
            error = $_.Exception.Message
            product_manifest_uri = $productUri
            package_manifest_uri = $manifestUri
        }
    }
}

$result | ConvertTo-Json -Depth 8
