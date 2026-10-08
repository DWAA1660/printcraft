# Test double for WIA: defines New-Object so `New-Object -ComObject WIA.DeviceManager` returns a
# fake device manager with one scanner (and a camera, which must not be listed), then runs the
# real script. Used by wia_script_drives_a_fake_device_manager when PowerShell 7 (pwsh) is installed.
param([string]$Script, [int]$Sheets = 3, [string]$Fail = '')
class FakeProp { [int]$PropertyID; $Value
  FakeProp([int]$id, $v) { $this.PropertyID = $id; $this.Value = $v } }
class FakeProps : System.Collections.IEnumerable {
  [System.Collections.ArrayList]$List = [System.Collections.ArrayList]::new()
  [System.Collections.IEnumerator] GetEnumerator() { return $this.List.GetEnumerator() }
  [object] Item($k) { foreach ($p in $this.List) { if ($p.PropertyID -eq $k -or $p.Name -eq $k) { return $p } }; throw "no $k" }
}
class FakeImage { [byte[]]$Data
  SaveFile([string]$path) { [System.IO.File]::WriteAllBytes($path, $this.Data) } }
class FakeItem { [FakeProps]$Properties; $Dev
  [object] Transfer([string]$fmt) {
    if ($global:Fail) { throw [System.Runtime.InteropServices.COMException]::new('Busy device', [int]0x80210006) }
    $sel = ($this.Dev.Properties.Item(3088)).Value
    if (($sel -band 1) -and $global:Left -le 0) { throw [System.Runtime.InteropServices.COMException]::new('There are no documents in the document feeder.', [int]0x80210003) }
    if ($sel -band 1) { $global:Left--; ($this.Dev.Properties.Item(3087)).Value = [int]($global:Left -gt 0) }
    $img = [FakeImage]::new(); $img.Data = [byte[]](0x89,0x50,0x4E,0x47); return $img }
}
class FakeItems { $It; [object] Item([int]$i) { return $this.It } }
class FakeDev { [FakeProps]$Properties; [FakeItems]$Items }
class FakeInfo { [int]$Type = 1; [string]$DeviceID; [FakeProps]$Properties; $Dev
  [object] Connect() { return $this.Dev } }
$global:Left = $Sheets
$global:Fail = $Fail
function New-Object { param([string]$ComObject)
  $dp = [FakeProps]::new(); foreach ($id in 3087,3088,3096) { [void]$dp.List.Add([FakeProp]::new($id, 0)) }
  ($dp.Item(3087)).Value = [int]($global:Left -gt 0)
  $ip = [FakeProps]::new(); foreach ($id in 6146,6147,6148,6151,6152) { [void]$ip.List.Add([FakeProp]::new($id, 0)) }
  $dev = [FakeDev]::new(); $dev.Properties = $dp
  $item = [FakeItem]::new(); $item.Properties = $ip; $item.Dev = $dev
  $items = [FakeItems]::new(); $items.It = $item; $dev.Items = $items
  $global:LastItem = $item
  $info = [FakeInfo]::new(); $info.DeviceID = '{6BDD}\0001'; $info.Dev = $dev
  $np = [FakeProps]::new(); [void]$np.List.Add([pscustomobject]@{ PropertyID = 7; Name = 'Name'; Value = 'Fake WIA Scanner' }); $info.Properties = $np
  $other = [FakeInfo]::new(); $other.Type = 2; $other.DeviceID = 'camera'
  return [pscustomobject]@{ DeviceInfos = @($info, $other) }
}
$rest = $args
& $Script @rest
"PROPS`t" + (($global:LastItem.Properties.List | % { "$($_.PropertyID)=$($_.Value)" }) -join ' ')
