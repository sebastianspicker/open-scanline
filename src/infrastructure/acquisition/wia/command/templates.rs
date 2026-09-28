macro_rules! wia_property_helpers {
    () => {
        r#"
function Get-WiaProperty($owner, [int]$id) {
  try { return $owner.Properties.Item($id) } catch { return $null }
}

function Get-WiaItemCategory($candidate) {
  $property = Get-WiaProperty $candidate $WiaIpaItemCategory
  if ($null -eq $property) { return '' }
  return ([string]$property.Value).Trim('{}').ToLowerInvariant()
}
"#
    };
}

pub(super) const SINGLE_TRANSFER: &str = concat!(
    r#"
$ErrorActionPreference = 'Stop'
$out = '__OSL_OUTPUT__'
$want = '__OSL_RAW_ID__'
$wantMode = '__OSL_SOURCE__'
$wantDuplex = __OSL_DUPLEX__

# WIA 2.0 item category property and documented category GUIDs.
$WiaIpaItemCategory = 4125
$WiaCategoryFlatbed = 'fb607b1f-43f3-488b-855b-fb703ec342a6'
$WiaCategoryFeeder = 'fe131934-f84c-42ad-8da4-6129cddd7288'
$WiaCategoryFeederFront = '4823175c-3b28-487b-a7e6-eebc17614fd1'
$WiaCategoryFeederBack = '61ca74d4-39db-42aa-89b1-8c19c9cd4c23'
$WiaCategoryFilm = 'fcf65be7-3ce3-4473-af85-f5d37d21b68a'

# WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES / _SELECT and their flag values.
$WiaDpsDocumentHandlingCapabilities = 3086
$WiaDpsDocumentHandlingSelect = 3088
$WiaDocumentHandlingFeeder = 1
$WiaDocumentHandlingDuplex = 4
$WiaDocumentHandlingFrontOnly = 32
"#,
    wia_property_helpers!(),
    r#"

function Get-WiaItemText($candidate) {
  $parts = @([string]$candidate.Name)
  foreach ($propertyId in @(4098, 4099)) {
    $property = Get-WiaProperty $candidate $propertyId
    if ($null -ne $property) { $parts += [string]$property.Value }
  }
  return ($parts -join ' ').ToLowerInvariant()
}

function Find-WiaItem($items, [string[]]$categories, [string[]]$names) {
  foreach ($candidate in $items) {
    if ($categories -contains (Get-WiaItemCategory $candidate)) { return $candidate }
  }
  foreach ($candidate in $items) {
    $text = Get-WiaItemText $candidate
    foreach ($name in $names) {
      if ($text.Contains($name)) { return $candidate }
    }
  }
  return $null
}

$dm = New-Object -ComObject WIA.DeviceManager
$dev = $null
foreach ($info in $dm.DeviceInfos) {
  if ($info.DeviceID -eq $want) { $dev = $info.Connect(); break }
}
if ($null -eq $dev) { throw 'device not connected' }

$items = @()
for ($index = 1; $index -le $dev.Items.Count; $index++) { $items += $dev.Items.Item($index) }
if ($wantMode -eq 'document') {
  $item = Find-WiaItem $items @($WiaCategoryFeeder, $WiaCategoryFeederFront, $WiaCategoryFeederBack) @('automatic document feeder', 'adf', 'feeder', 'document')
  if ($null -eq $item) { throw 'WIA device does not expose a document feeder item' }
} elseif ($wantMode -eq 'film') {
  $item = Find-WiaItem $items @($WiaCategoryFilm) @('transparency', 'tpu', 'film', 'negative', 'slide')
  if ($null -eq $item) { throw 'WIA device does not expose a film item' }
} else {
  $item = Find-WiaItem $items @($WiaCategoryFlatbed) @('flatbed', 'platen', 'reflective')
  if ($null -eq $item) {
    if ($items.Count -eq 1) {
      # A single unknown item is the only conservative reflective fallback.
      $item = $items[0]
    } else {
      throw 'WIA device does not expose a reflective item'
    }
  }
}

if ($wantDuplex -and $wantMode -ne 'document') { throw 'duplex is only valid with a document feeder source' }
if ($wantMode -eq 'document') {
  $selectProperty = Get-WiaProperty $dev $WiaDpsDocumentHandlingSelect
  if ($null -eq $selectProperty) { $selectProperty = Get-WiaProperty $item $WiaDpsDocumentHandlingSelect }
  if ($null -ne $selectProperty) {
    $documentHandling = ([int]$selectProperty.Value -bor $WiaDocumentHandlingFeeder)
    if ($wantDuplex) {
      $capabilities = Get-WiaProperty $dev $WiaDpsDocumentHandlingCapabilities
      if ($null -eq $capabilities) { $capabilities = Get-WiaProperty $item $WiaDpsDocumentHandlingCapabilities }
      if ($null -eq $capabilities -or (([int]$capabilities.Value -band $WiaDocumentHandlingDuplex) -eq 0)) {
        throw 'WIA device does not advertise duplex acquisition'
      }
      $documentHandling = (($documentHandling -bor $WiaDocumentHandlingDuplex) -band (-bnot $WiaDocumentHandlingFrontOnly))
    }
    $selectProperty.Value = $documentHandling
  } elseif ($wantDuplex) {
    throw 'WIA device does not expose document handling controls for duplex acquisition'
  }
}

# WIA_IPS_CUR_INTENT, XRES, YRES, XPOS, YPOS, XEXTENT, and YEXTENT.
$item.Properties.Item(6146).Value = __OSL_INTENT__
$item.Properties.Item(6147).Value = __OSL_DPI_X__
$item.Properties.Item(6148).Value = __OSL_DPI_Y__
$item.Properties.Item(6149).Value = __OSL_X__
$item.Properties.Item(6150).Value = __OSL_Y__
$item.Properties.Item(6151).Value = __OSL_WIDTH__
$item.Properties.Item(6152).Value = __OSL_HEIGHT__
$img = $item.Transfer()
$img.SaveFile($out)
"#
);
pub(super) const PAGE_TRANSFER: &str = concat!(
    r#"
$ErrorActionPreference = 'Stop'
$outDir = '__OSL_OUTPUT__'
$want = '__OSL_RAW_ID__'
$wantDuplex = __OSL_DUPLEX__
$wantSides = __OSL_MAX_PAGES__

$WiaIpaItemCategory = 4125
$WiaCategoryFeeder = 'fe131934-f84c-42ad-8da4-6129cddd7288'
$WiaCategoryFeederFront = '4823175c-3b28-487b-a7e6-eebc17614fd1'
$WiaCategoryFeederBack = '61ca74d4-39db-42aa-89b1-8c19c9cd4c23'
$WiaDpsDocumentHandlingCapabilities = 3086
$WiaDpsDocumentHandlingSelect = 3088
$WiaDpsPages = 3096
$WiaDocumentHandlingFeeder = 1
$WiaDocumentHandlingDuplex = 4
$WiaDocumentHandlingFrontFirst = 8
$WiaDocumentHandlingFrontOnly = 32
"#,
    wia_property_helpers!(),
    r#"

function Find-WiaFeeder($items) {
  foreach ($candidate in $items) {
    if (@($WiaCategoryFeeder, $WiaCategoryFeederFront, $WiaCategoryFeederBack) -contains (Get-WiaItemCategory $candidate)) { return $candidate }
  }
  foreach ($candidate in $items) {
    $description = Get-WiaProperty $candidate 4098
    $descriptionText = if ($null -eq $description) { '' } else { [string]$description.Value }
    $text = (([string]$candidate.Name) + ' ' + $descriptionText).ToLowerInvariant()
    if ($text.Contains('feeder') -or $text.Contains('adf') -or $text.Contains('document')) { return $candidate }
  }
  return $null
}

function Test-WiaFeederExhausted($errorRecord) {
  $text = [string]$errorRecord
  return $text -match '(?i)(paper|document|feeder).{0,40}(empty|emptying|out|none|no more)|no.{0,40}(paper|document|feeder)|wia_error_paper_empty'
}

$dm = New-Object -ComObject WIA.DeviceManager
$dev = $null
foreach ($info in $dm.DeviceInfos) {
  if ($info.DeviceID -eq $want) { $dev = $info.Connect(); break }
}
if ($null -eq $dev) { throw 'device not connected' }

$items = @()
for ($index = 1; $index -le $dev.Items.Count; $index++) { $items += $dev.Items.Item($index) }
$item = Find-WiaFeeder $items
if ($null -eq $item) { throw 'WIA device does not expose a document feeder item' }

$selectProperty = Get-WiaProperty $dev $WiaDpsDocumentHandlingSelect
if ($null -eq $selectProperty) { $selectProperty = Get-WiaProperty $item $WiaDpsDocumentHandlingSelect }
if ($null -eq $selectProperty) { throw 'WIA device does not expose document handling controls' }
$capabilities = Get-WiaProperty $dev $WiaDpsDocumentHandlingCapabilities
if ($null -eq $capabilities) { $capabilities = Get-WiaProperty $item $WiaDpsDocumentHandlingCapabilities }
$documentHandling = ([int]$selectProperty.Value -bor $WiaDocumentHandlingFeeder)
if ($null -ne $capabilities -and (([int]$capabilities.Value -band $WiaDocumentHandlingFrontFirst) -ne 0)) {
  $documentHandling = ($documentHandling -bor $WiaDocumentHandlingFrontFirst)
}
if ($wantDuplex) {
  if ($null -eq $capabilities -or (([int]$capabilities.Value -band $WiaDocumentHandlingDuplex) -eq 0)) {
    throw 'WIA device does not advertise duplex acquisition'
  }
  $documentHandling = (($documentHandling -bor $WiaDocumentHandlingDuplex) -band (-bnot $WiaDocumentHandlingFrontOnly))
}
$selectProperty.Value = $documentHandling

$pagesProperty = Get-WiaProperty $dev $WiaDpsPages
if ($null -eq $pagesProperty) { $pagesProperty = Get-WiaProperty $item $WiaDpsPages }
if ($null -ne $pagesProperty) { $pagesProperty.Value = $wantSides }

$item.Properties.Item(6146).Value = __OSL_INTENT__
$item.Properties.Item(6147).Value = __OSL_DPI_X__
$item.Properties.Item(6148).Value = __OSL_DPI_Y__
$item.Properties.Item(6149).Value = __OSL_X__
$item.Properties.Item(6150).Value = __OSL_Y__
$item.Properties.Item(6151).Value = __OSL_WIDTH__
$item.Properties.Item(6152).Value = __OSL_HEIGHT__

for ($side = 1; $side -le $wantSides; $side++) {
  try {
    $path = Join-Path $outDir ('page_{0:D6}.png' -f $side)
    $img = $item.Transfer()
    $img.SaveFile($path)
  } catch {
    if (Test-WiaFeederExhausted $_) { Write-Output 'OSL_FEEDER_EXHAUSTED'; break }
    throw
  }
}
"#
);
