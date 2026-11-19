//! System management queries answered by Windows' own management tools
//! (CIM/WMI through PowerShell, powercfg). Each runs in a background job.

use crate::util::{powershell, ps_json, ps_object, ps_quote, run_hidden};
use serde_json::Value;

pub type Rows = Vec<Value>;

pub fn services() -> Result<Rows, String> {
    ps_json("Get-CimInstance Win32_Service | Select-Object Name,DisplayName,State,StartMode,DelayedAutoStart,StartName,ProcessId,ServiceType,ExitCode,PathName,Description")
}

pub fn service_action(name: &str, action: &str) -> Result<String, String> {
    let n = ps_quote(name);
    let script = match action {
        "start" => format!("Start-Service -Name {n} -ErrorAction Stop; 'Started'"),
        "stop" => format!("Stop-Service -Name {n} -Force -ErrorAction Stop; 'Stopped'"),
        "restart" => format!("Restart-Service -Name {n} -Force -ErrorAction Stop; 'Restarted'"),
        "auto" => format!("Set-Service -Name {n} -StartupType Automatic -ErrorAction Stop; 'Set to automatic'"),
        "manual" => format!("Set-Service -Name {n} -StartupType Manual -ErrorAction Stop; 'Set to manual'"),
        "disabled" => format!("Set-Service -Name {n} -StartupType Disabled -ErrorAction Stop; 'Disabled'"),
        _ => return Err("Unknown action".into()),
    };
    let out = powershell(&format!("try {{ {script} }} catch {{ 'ERROR: ' + $_.Exception.Message }}"))?;
    if out.contains("ERROR:") {
        Err(out.trim().to_string())
    } else {
        Ok(out.trim().to_string())
    }
}

pub fn dependent_services(name: &str) -> Result<String, String> {
    let n = ps_quote(name);
    powershell(&format!(
        "$s=Get-Service -Name {n} -EA SilentlyContinue; if($s){{ 'Depends on: ' + (($s.ServicesDependedOn|%{{$_.Name}}) -join ', '); 'Needed by: ' + (($s.DependentServices|%{{$_.Name}}) -join ', ') }}"
    ))
}

const STARTUP_SCRIPT: &str = r#"
$out=@()
$locs=@(
 @{H='HKCU';P='Software\Microsoft\Windows\CurrentVersion\Run';A='HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'},
 @{H='HKLM';P='SOFTWARE\Microsoft\Windows\CurrentVersion\Run';A='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'},
 @{H='HKLM';P='SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run';A='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32'},
 @{H='HKCU';P='Software\Microsoft\Windows\CurrentVersion\RunOnce';A=''},
 @{H='HKLM';P='SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce';A=''}
)
foreach($l in $locs){
 $k=Get-Item -LiteralPath ($l.H+':\'+$l.P) -EA SilentlyContinue
 if($k){ foreach($n in $k.GetValueNames()){ if($n -eq ''){continue}
   $st='Enabled'
   if($l.A -ne ''){ $a=(Get-ItemProperty -LiteralPath $l.A -Name $n -EA SilentlyContinue).$n; if($a -and ($a[0] -band 1)){$st='Disabled'} }
   $cmd=[string]$k.GetValue($n)
   $exe=$cmd; if($cmd -match '^"([^"]+)"'){$exe=$matches[1]} elseif($cmd -match '^(\S+\.exe)'){$exe=$matches[1]}
   $size=0; $pub=''; if(Test-Path -LiteralPath $exe -EA SilentlyContinue){ $fi=Get-Item -LiteralPath $exe -EA SilentlyContinue; $size=$fi.Length; $pub=$fi.VersionInfo.CompanyName }
   $out+=[pscustomobject]@{Name=$n;Command=$cmd;Location=($l.H+'\'+$l.P);Approved=$l.A;State=$st;Kind='Registry';Publisher=$pub;FileSize=$size;Executable=$exe}
 } }
}
$folders=@(@{F=[Environment]::GetFolderPath('Startup');A='HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder'},@{F=[Environment]::GetFolderPath('CommonStartup');A='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder'})
foreach($f in $folders){ Get-ChildItem -LiteralPath $f.F -File -EA SilentlyContinue | ?{$_.Name -ne 'desktop.ini'} | %{
  $st='Enabled'; $a=(Get-ItemProperty -LiteralPath $f.A -Name $_.Name -EA SilentlyContinue).($_.Name); if($a -and ($a[0] -band 1)){$st='Disabled'}
  $out+=[pscustomobject]@{Name=$_.Name;Command=$_.FullName;Location=$f.F;Approved=$f.A;State=$st;Kind='Startup folder';Publisher='';FileSize=$_.Length;Executable=$_.FullName}
} }
Get-ScheduledTask -EA SilentlyContinue | ?{ $_.Triggers | ?{ $_.CimClass.CimClassName -in 'MSFT_TaskLogonTrigger','MSFT_TaskBootTrigger' } } | ?{ $_.TaskPath -notlike '\Microsoft\*' } | %{
  $out+=[pscustomobject]@{Name=$_.TaskName;Command=(($_.Actions|%{$_.Execute+' '+$_.Arguments}) -join '; ');Location=('Task '+$_.TaskPath);Approved='';State=[string]$_.State;Kind='Scheduled task';Publisher=$_.Author;FileSize=0;Executable=''}
}
Get-CimInstance Win32_Service -Filter "StartMode='Auto'" -EA SilentlyContinue | ?{ $_.PathName -and $_.PathName -notmatch 'Windows\\(System32|SysWOW64)\\svchost' -and $_.PathName -notmatch '\\Windows\\' } | %{
  $out+=[pscustomobject]@{Name=$_.DisplayName;Command=$_.PathName;Location='Service';Approved='';State=[string]$_.State;Kind='Auto-start service';Publisher='';FileSize=0;Executable=''}
}
$out
"#;

pub fn startup() -> Result<Rows, String> {
    ps_json(STARTUP_SCRIPT)
}

pub fn startup_toggle(approved_key: &str, name: &str, enable: bool) -> Result<String, String> {
    if approved_key.is_empty() {
        return Err("This kind of entry can't be switched off here.".into());
    }
    let first = if enable { 2 } else { 3 };
    let script = format!(
        "try {{ if(!(Test-Path -LiteralPath {k})){{ New-Item -Path {k} -Force | Out-Null }}; New-ItemProperty -LiteralPath {k} -Name {n} -PropertyType Binary -Value ([byte[]]({first},0,0,0,0,0,0,0,0,0,0,0)) -Force -ErrorAction Stop | Out-Null; 'OK' }} catch {{ 'ERROR: ' + $_.Exception.Message }}",
        k = ps_quote(approved_key),
        n = ps_quote(name)
    );
    let out = powershell(&script)?;
    if out.contains("ERROR:") {
        Err(out.trim().into())
    } else {
        Ok(if enable { "Enabled".into() } else { "Disabled".into() })
    }
}

pub fn tasks() -> Result<Rows, String> {
    ps_json("Get-ScheduledTask | Select-Object TaskName,TaskPath,@{n='State';e={[string]$_.State}},Author,Description,@{n='Actions';e={($_.Actions|%{$_.Execute+' '+$_.Arguments}) -join '; '}},@{n='Triggers';e={($_.Triggers|%{$_.CimClass.CimClassName -replace 'MSFT_Task','' -replace 'Trigger',''}) -join ', '}},@{n='RunAs';e={$_.Principal.UserId}},@{n='RunLevel';e={[string]$_.Principal.RunLevel}}")
}

pub fn task_info(path: &str, name: &str) -> Result<Value, String> {
    ps_object(&format!(
        "Get-ScheduledTaskInfo -TaskPath {} -TaskName {} | Select-Object LastRunTime,LastTaskResult,NextRunTime,NumberOfMissedRuns",
        ps_quote(path),
        ps_quote(name)
    ))
}

pub fn task_action(path: &str, name: &str, action: &str) -> Result<String, String> {
    let cmd = match action {
        "run" => "Start-ScheduledTask",
        "end" => "Stop-ScheduledTask",
        "enable" => "Enable-ScheduledTask",
        "disable" => "Disable-ScheduledTask",
        _ => return Err("Unknown action".into()),
    };
    let out = powershell(&format!(
        "try {{ {cmd} -TaskPath {} -TaskName {} -ErrorAction Stop | Out-Null; 'OK' }} catch {{ 'ERROR: ' + $_.Exception.Message }}",
        ps_quote(path),
        ps_quote(name)
    ))?;
    if out.contains("ERROR:") {
        Err(out.trim().into())
    } else {
        Ok("Done".into())
    }
}

pub fn drivers() -> Result<Rows, String> {
    ps_json("Get-CimInstance Win32_SystemDriver | Select-Object Name,DisplayName,State,StartMode,ServiceType,PathName,Description,ErrorControl")
}

pub fn signed_drivers() -> Result<Rows, String> {
    ps_json("Get-CimInstance Win32_PnPSignedDriver | ?{$_.DeviceName} | Select-Object DeviceName,DeviceClass,DriverVersion,@{n='DriverDate';e={if($_.DriverDate){$_.DriverDate.ToString('yyyy-MM-dd')}}},DriverProviderName,IsSigned,Signer,InfName,Manufacturer")
}

pub fn devices() -> Result<Rows, String> {
    ps_json("Get-CimInstance Win32_PnPEntity | Select-Object Name,PNPClass,Manufacturer,Status,ConfigManagerErrorCode,Present,Service,DeviceID")
}

pub fn device_problem(code: i64) -> &'static str {
    match code {
        0 => "Working properly",
        1 => "Not configured correctly",
        3 => "Driver may be corrupted",
        10 => "Cannot start",
        12 => "Not enough free resources",
        14 => "Needs a restart",
        18 => "Drivers need reinstalling",
        19 => "Registry information incomplete",
        21 => "Being removed",
        22 => "Disabled",
        24 => "Not present / not working",
        28 => "Drivers not installed",
        29 => "Disabled by firmware",
        31 => "Driver could not load",
        32 => "Driver disabled",
        39 => "Driver missing or corrupted",
        43 => "Stopped after reporting problems",
        45 => "Not connected",
        52 => "Driver signature could not be verified",
        _ => "Problem",
    }
}

pub struct PowerInfo {
    pub plans: Vec<(String, String, bool)>,
    pub sleep_states: String,
    pub battery: Rows,
    pub settings: Rows,
}

pub fn power() -> Result<PowerInfo, String> {
    let list = run_hidden("powercfg", &["/list"])?;
    let mut plans = vec![];
    for line in list.lines() {
        if let Some(pos) = line.find(':') {
            let rest = line[pos + 1..].trim();
            let mut parts = rest.splitn(2, ' ');
            let guid = parts.next().unwrap_or("").trim().to_string();
            if guid.len() == 36 && guid.matches('-').count() == 4 {
                let tail = parts.next().unwrap_or("");
                let name = tail.trim().trim_end_matches('*').trim().trim_start_matches('(').trim_end_matches(')').to_string();
                plans.push((guid, name, line.trim_end().ends_with('*')));
            }
        }
    }
    let sleep_states = run_hidden("powercfg", &["/a"]).unwrap_or_default();
    let battery = ps_json(
        "Get-CimInstance Win32_Battery | Select-Object Name,EstimatedChargeRemaining,@{n='Status';e={switch($_.BatteryStatus){1{'Discharging'}2{'On AC power'}3{'Fully charged'}4{'Low'}5{'Critical'}6{'Charging'}7{'Charging (high)'}8{'Charging (low)'}9{'Charging (critical)'}default{'Unknown'}}}},EstimatedRunTime,DesignVoltage,Chemistry",
    )
    .unwrap_or_default();
    let settings = ps_json(
        r#"$g=(powercfg /getactivescheme) -replace '.*:\s*(\S+).*','$1'
$items=@(@('SUB_SLEEP','STANDBYIDLE','Sleep after (seconds)'),@('SUB_SLEEP','HIBERNATEIDLE','Hibernate after (seconds)'),@('SUB_VIDEO','VIDEOIDLE','Turn off display after (seconds)'),@('SUB_PROCESSOR','PROCTHROTTLEMAX','Maximum processor state %'),@('SUB_PROCESSOR','PROCTHROTTLEMIN','Minimum processor state %'),@('SUB_DISK','DISKIDLE','Turn off disk after (seconds)'))
foreach($i in $items){ $o=powercfg /query $g $i[0] $i[1] 2>$null; $ac=($o|Select-String 'AC Power Setting Index'|%{[convert]::ToInt64(($_ -split ':')[1].Trim(),16)}); $dc=($o|Select-String 'DC Power Setting Index'|%{[convert]::ToInt64(($_ -split ':')[1].Trim(),16)}); [pscustomobject]@{Setting=$i[2];PluggedIn=$ac;OnBattery=$dc} }"#,
    )
    .unwrap_or_default();
    Ok(PowerInfo { plans, sleep_states, battery, settings })
}

pub fn set_power_plan(guid: &str) -> Result<String, String> {
    run_hidden("powercfg", &["/setactive", guid]).map(|_| "Power plan changed".into())
}

pub fn battery_report() -> Result<std::path::PathBuf, String> {
    let p = crate::util::sub_dir("reports").join(format!("battery-report-{}.html", crate::util::stamp()));
    run_hidden("powercfg", &["/batteryreport", "/output", &p.to_string_lossy()])?;
    Ok(p)
}

pub fn adapters() -> Result<Value, String> {
    ps_object(
        r#"[pscustomobject]@{
 Adapters=@(Get-NetAdapter -IncludeHidden:$false -EA SilentlyContinue | Select-Object Name,InterfaceDescription,Status,LinkSpeed,MacAddress,MediaType,PhysicalMediaType,DriverVersion,DriverProvider,@{n='DriverDate';e={[string]$_.DriverDate}},ifIndex,MtuSize,VlanID,FullDuplex,Virtual)
 Addresses=@(Get-NetIPAddress -EA SilentlyContinue | ?{$_.IPAddress -notlike 'fe80*' -or $true} | Select-Object InterfaceAlias,IPAddress,PrefixLength,@{n='Family';e={[string]$_.AddressFamily}},@{n='Origin';e={[string]$_.PrefixOrigin}},@{n='State';e={[string]$_.AddressState}})
 Dns=@(Get-DnsClientServerAddress -EA SilentlyContinue | ?{$_.ServerAddresses} | Select-Object InterfaceAlias,@{n='Family';e={if($_.AddressFamily -eq 2){'IPv4'}else{'IPv6'}}},@{n='Servers';e={$_.ServerAddresses -join ', '}})
 Routes=@(Get-NetRoute -EA SilentlyContinue | ?{$_.DestinationPrefix -eq '0.0.0.0/0' -or $_.DestinationPrefix -eq '::/0'} | Select-Object InterfaceAlias,DestinationPrefix,NextHop,RouteMetric)
 Profiles=@(Get-NetConnectionProfile -EA SilentlyContinue | Select-Object Name,InterfaceAlias,@{n='Category';e={[string]$_.NetworkCategory}},@{n='IPv4';e={[string]$_.IPv4Connectivity}})
}"#,
    )
}

pub fn adapter_action(name: &str, enable: bool) -> Result<String, String> {
    let cmd = if enable { "Enable-NetAdapter" } else { "Disable-NetAdapter" };
    let out = powershell(&format!(
        "try {{ {cmd} -Name {} -Confirm:$false -ErrorAction Stop; 'OK' }} catch {{ 'ERROR: ' + $_.Exception.Message }}",
        ps_quote(name)
    ))?;
    if out.contains("ERROR:") {
        Err(out.trim().into())
    } else {
        Ok("Done".into())
    }
}

pub fn system_info() -> Result<Value, String> {
    ps_object(
        r#"$cs=Get-CimInstance Win32_ComputerSystem; $os=Get-CimInstance Win32_OperatingSystem; $b=Get-CimInstance Win32_BIOS; $bb=Get-CimInstance Win32_BaseBoard
[pscustomobject]@{
 Computer=[pscustomobject]@{Name=$cs.Name;Manufacturer=$cs.Manufacturer;Model=$cs.Model;SystemType=$cs.SystemType;Domain=$cs.Domain;PartOfDomain=$cs.PartOfDomain;HypervisorPresent=$cs.HypervisorPresent;TotalMemory=$cs.TotalPhysicalMemory;Processors=$cs.NumberOfProcessors;LogicalProcessors=$cs.NumberOfLogicalProcessors;PCSystemType=$cs.PCSystemType}
 OS=[pscustomobject]@{Caption=$os.Caption;Version=$os.Version;Build=$os.BuildNumber;Architecture=$os.OSArchitecture;InstallDate=[string]$os.InstallDate;LastBoot=[string]$os.LastBootUpTime;SystemDrive=$os.SystemDrive;WindowsFolder=$os.WindowsDirectory;Locale=$os.Locale;Language=$os.OSLanguage;CodeSet=$os.CodeSet;Organization=$os.Organization;SerialNumber=$os.SerialNumber;MaxProcessMemory=$os.MaxProcessMemorySize;Encryption=$os.EncryptionLevel}
 Bios=[pscustomobject]@{Vendor=$b.Manufacturer;Version=$b.SMBIOSBIOSVersion;ReleaseDate=[string]$b.ReleaseDate;SMBIOS=("{0}.{1}" -f $b.SMBIOSMajorVersion,$b.SMBIOSMinorVersion);Board=($bb.Manufacturer+' '+$bb.Product);BoardVersion=$bb.Version}
 Cpu=@(Get-CimInstance Win32_Processor | Select-Object Name,Manufacturer,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed,CurrentClockSpeed,L2CacheSize,L3CacheSize,SocketDesignation,VirtualizationFirmwareEnabled,SecondLevelAddressTranslationExtensions,AddressWidth,Revision,ProcessorId)
 Memory=@(Get-CimInstance Win32_PhysicalMemory | Select-Object BankLabel,DeviceLocator,Capacity,Speed,ConfiguredClockSpeed,Manufacturer,PartNumber,FormFactor,SMBIOSMemoryType,ConfiguredVoltage)
 MemoryArray=@(Get-CimInstance Win32_PhysicalMemoryArray | Select-Object MaxCapacity,MemoryDevices)
 Gpu=@(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,@{n='DriverDate';e={[string]$_.DriverDate}},AdapterRAM,VideoProcessor,CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate,VideoModeDescription,Status)
 Disks=@(Get-CimInstance Win32_DiskDrive | Select-Object Model,Size,InterfaceType,MediaType,Partitions,FirmwareRevision,SerialNumber,BytesPerSector,Status)
 Monitors=@(Get-CimInstance -Namespace root\wmi -ClassName WmiMonitorID -EA SilentlyContinue | %{ [pscustomobject]@{Name=(($_.UserFriendlyName|?{$_ -ne 0}|%{[char]$_}) -join '');Maker=(($_.ManufacturerName|?{$_ -ne 0}|%{[char]$_}) -join '');Year=$_.YearOfManufacture} })
 Sound=@(Get-CimInstance Win32_SoundDevice | Select-Object Name,Manufacturer,Status)
}"#,
    )
}

pub fn windows_config() -> Result<Value, String> {
    ps_object(
        r#"function Get-RegVal($p,$n){ try { (Get-ItemProperty -LiteralPath $p -Name $n -EA Stop).$n } catch { $null } }
$mp=$null; try { $mp=Get-MpComputerStatus -EA Stop } catch {}
$fw=@(); try { $fw=@(Get-NetFirewallProfile -EA Stop | Select-Object Name,@{n='Enabled';e={if([string]$_.Enabled -in 'True','1'){'Yes'}else{'No'}}},@{n='DefaultInboundAction';e={[string]$_.DefaultInboundAction}},@{n='DefaultOutboundAction';e={[string]$_.DefaultOutboundAction}}) } catch {}
$dg=$null; try { $dg=Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard -EA Stop } catch {}
$sb=$null; try { $sb=Confirm-SecureBootUEFI -EA Stop } catch { $sb='Unknown (needs Admin view or legacy BIOS)' }
$bl=@(); try { $bl=@(Get-BitLockerVolume -EA Stop | Select-Object MountPoint,ProtectionStatus,EncryptionPercentage,VolumeStatus) } catch {}
[pscustomobject]@{
 Security=[pscustomobject]@{
  UacEnabled=(Get-RegVal 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' 'EnableLUA')
  UacPromptLevel=(Get-RegVal 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' 'ConsentPromptBehaviorAdmin')
  SecureBoot=$sb
  DefenderRealTime=$(if($mp){$mp.RealTimeProtectionEnabled}else{'Not available'})
  DefenderSignatures=$(if($mp){[string]$mp.AntivirusSignatureLastUpdated}else{''})
  TamperProtection=$(if($mp){$mp.IsTamperProtected}else{''})
  VirtualizationBasedSecurity=$(if($dg){switch($dg.VirtualizationBasedSecurityStatus){0{'Off'}1{'Enabled, not running'}2{'Running'}default{'?'}}}else{'Unknown'})
  CredentialGuard=$(if($dg){($dg.SecurityServicesRunning -contains 1)}else{''})
  MemoryIntegrity=$(if($dg){($dg.SecurityServicesRunning -contains 2)}else{''})
  SmartScreen=(Get-RegVal 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer' 'SmartScreenEnabled')
 }
 Firewall=$fw
 BitLocker=$bl
 System=[pscustomobject]@{
  FastStartup=(Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' 'HiberbootEnabled')
  Hibernation=(Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\Power' 'HibernateEnabled')
  GameMode=(Get-RegVal 'HKCU:\Software\Microsoft\GameBar' 'AutoGameModeEnabled')
  HardwareGpuScheduling=(Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\GraphicsDrivers' 'HwSchMode')
  VisualEffects=(Get-RegVal 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects' 'VisualFXSetting')
  DarkModeApps=(Get-RegVal 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' 'AppsUseLightTheme')
  Transparency=(Get-RegVal 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' 'EnableTransparency')
  LongPaths=(Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'LongPathsEnabled')
  DeveloperMode=(Get-RegVal 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' 'AllowDevelopmentWithoutDevLicense')
  RemoteDesktop=$(if((Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server' 'fDenyTSConnections') -eq 0){'Allowed'}else{'Off'})
  PowerThrottling=(Get-RegVal 'HKLM:\SYSTEM\CurrentControlSet\Control\Power\PowerThrottling' 'PowerThrottlingOff')
  TimeZone=[string](Get-TimeZone -EA SilentlyContinue).DisplayName
  UpdateService=[string](Get-Service wuauserv -EA SilentlyContinue).Status
 }
 PageFiles=@(Get-CimInstance Win32_PageFileUsage | Select-Object Name,AllocatedBaseSize,CurrentUsage,PeakUsage)
 Updates=@(Get-HotFix -EA SilentlyContinue | Sort-Object InstalledOn -Descending -EA SilentlyContinue | Select-Object -First 15 HotFixID,Description,@{n='InstalledOn';e={[string]$_.InstalledOn}})
 Environment=@(Get-ChildItem Env: | Select-Object Name,Value)
 Features=@(try { Get-WindowsOptionalFeature -Online -EA Stop | ?{$_.State -eq 'Enabled'} | Select-Object FeatureName } catch { })
}"#,
    )
}

pub fn events(log: &str, levels: &[u8], hours: u32, max: u32) -> Result<Rows, String> {
    let lv = levels.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(",");
    ps_json(&format!(
        "Get-WinEvent -FilterHashtable @{{LogName={};Level={lv};StartTime=(Get-Date).AddHours(-{hours})}} -MaxEvents {max} -EA SilentlyContinue | Select-Object @{{n='Time';e={{$_.TimeCreated.ToString('yyyy-MM-dd HH:mm:ss')}}}},@{{n='Unix';e={{[int64](($_.TimeCreated.ToUniversalTime()-[datetime]'1970-01-01').TotalSeconds)}}}},LogName,ProviderName,Id,@{{n='Level';e={{$_.LevelDisplayName}}}},@{{n='Message';e={{ $m=$_.Message; if($m){{ $m=($m -split \"`n\")[0]; if($m.Length -gt 300){{$m.Substring(0,300)}}else{{$m}} }} }}}}",
        ps_quote(log)
    ))
}

pub fn event_logs() -> Result<Rows, String> {
    ps_json("Get-WinEvent -ListLog * -EA SilentlyContinue | ?{$_.RecordCount -gt 0} | Sort-Object RecordCount -Descending | Select-Object -First 80 LogName,RecordCount,@{n='SizeMB';e={[math]::Round($_.FileSize/1MB,1)}},IsEnabled")
}

pub fn boot_performance() -> Result<Rows, String> {
    ps_json("Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-Diagnostics-Performance/Operational';Id=100} -MaxEvents 10 -EA SilentlyContinue | %{ $x=[xml]$_.ToXml(); $d=@{}; $x.Event.EventData.Data | %{ $d[$_.Name]=$_.'#text' }; [pscustomobject]@{Time=$_.TimeCreated.ToString('yyyy-MM-dd HH:mm');BootMs=[int64]$d['BootTime'];MainPathMs=[int64]$d['MainPathBootTime'];PostBootMs=[int64]$d['BootPostBootTime'];Degraded=$d['IsDegradation']} }")
}

pub fn sensors() -> Result<Value, String> {
    ps_object(
        r#"$tz=@(); try { $tz=@(Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature -EA Stop | %{ [pscustomobject]@{Zone=$_.InstanceName;Celsius=[math]::Round($_.CurrentTemperature/10-273.15,1);CriticalC=[math]::Round($_.CriticalTripPoint/10-273.15,1)} }) } catch {}
$fans=@(Get-CimInstance Win32_Fan -EA SilentlyContinue | Select-Object Name,DesiredSpeed,Status)
$bs=@(); try { $bs=@(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus -EA Stop | Select-Object InstanceName,ChargeRate,DischargeRate,RemainingCapacity,Voltage,PowerOnline,Charging,Discharging) } catch {}
$full=@(); try { $full=@(Get-CimInstance -Namespace root/wmi -ClassName BatteryFullChargedCapacity -EA Stop | Select-Object InstanceName,FullChargedCapacity) } catch {}
$design=@(); try { $design=@(Get-CimInstance -Namespace root/wmi -ClassName BatteryStaticData -EA Stop | Select-Object InstanceName,DesignedCapacity,ManufactureName,SerialNumber) } catch {}
$nv=''; $smi=Get-Command nvidia-smi -EA SilentlyContinue; if($smi){ $nv=(& $smi.Source --query-gpu=name,temperature.gpu,fan.speed,power.draw,clocks.gr,clocks.mem,utilization.gpu,memory.used,memory.total --format=csv,noheader 2>$null) -join "`n" }
[pscustomobject]@{ThermalZones=$tz;Fans=$fans;BatteryStatus=$bs;FullCapacity=$full;DesignCapacity=$design;NvidiaSmi=$nv}"#,
    )
}

pub fn disk_health() -> Result<Value, String> {
    ps_object(
        r#"$pd=@(Get-PhysicalDisk -EA SilentlyContinue | Select-Object DeviceId,FriendlyName,@{n='MediaType';e={[string]$_.MediaType}},@{n='BusType';e={[string]$_.BusType}},@{n='Health';e={[string]$_.HealthStatus}},@{n='Operational';e={($_.OperationalStatus|%{[string]$_}) -join ', '}},Size,FirmwareVersion,SpindleSpeed,@{n='Usage';e={[string]$_.Usage}})
$rc=@(); try { $rc=@(Get-PhysicalDisk -EA Stop | Get-StorageReliabilityCounter -EA Stop | Select-Object DeviceId,Temperature,TemperatureMax,Wear,PowerOnHours,ReadErrorsTotal,ReadErrorsUncorrected,WriteErrorsTotal,WriteErrorsUncorrected,ReadLatencyMax,WriteLatencyMax,StartStopCycleCount,LoadUnloadCycleCount) } catch {}
$fp=@(); try { $fp=@(Get-CimInstance -Namespace root\wmi -ClassName MSStorageDriver_FailurePredictStatus -EA Stop | Select-Object InstanceName,PredictFailure,Reason) } catch {}
$vol=@(Get-Volume -EA SilentlyContinue | ?{$_.DriveLetter} | Select-Object DriveLetter,FileSystemLabel,FileSystem,@{n='Health';e={[string]$_.HealthStatus}},Size,SizeRemaining,@{n='DriveType';e={[string]$_.DriveType}})
$trim=(fsutil behavior query DisableDeleteNotify 2>$null) -join ' '
[pscustomobject]@{Disks=$pd;Reliability=$rc;FailurePredict=$fp;Volumes=$vol;Trim=$trim}"#,
    )
}

pub fn snapshot() -> Result<Value, String> {
    ps_object(
        r#"$apps=@(); foreach($p in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*','HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'){ $apps+=@(Get-ItemProperty $p -EA SilentlyContinue | ?{$_.DisplayName} | Select-Object @{n='Name';e={$_.DisplayName}},@{n='Version';e={$_.DisplayVersion}},Publisher) }
$runs=@(); foreach($k in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run','HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run'){ $i=Get-Item $k -EA SilentlyContinue; if($i){ foreach($n in $i.GetValueNames()){ if($n){ $runs+=[pscustomobject]@{Name=$n;Value=[string]$i.GetValue($n)} } } } }
$os=Get-CimInstance Win32_OperatingSystem
[pscustomobject]@{
 Taken=(Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
 Computer=$env:COMPUTERNAME
 OS=[pscustomobject]@{Name=$os.Caption;Version=$os.Version;Build=$os.BuildNumber}
 Services=@(Get-CimInstance Win32_Service | Select-Object Name,@{n='Value';e={$_.StartMode+' / '+$_.State}})
 Drivers=@(Get-CimInstance Win32_SystemDriver | Select-Object Name,@{n='Value';e={$_.StartMode+' / '+$_.State}})
 Startup=$runs
 Tasks=@(Get-ScheduledTask -EA SilentlyContinue | Select-Object @{n='Name';e={$_.TaskPath+$_.TaskName}},@{n='Value';e={[string]$_.State}})
 Programs=@($apps | Sort-Object Name -Unique | Select-Object Name,@{n='Value';e={[string]$_.Version}})
 Updates=@(Get-HotFix -EA SilentlyContinue | Select-Object @{n='Name';e={$_.HotFixID}},@{n='Value';e={[string]$_.InstalledOn}})
 Adapters=@(Get-NetAdapter -EA SilentlyContinue | Select-Object Name,@{n='Value';e={[string]$_.Status+' '+$_.LinkSpeed+' '+$_.MacAddress}})
 Devices=@(Get-CimInstance Win32_PnPEntity | ?{$_.Name} | Select-Object @{n='Name';e={$_.Name+' ['+$_.DeviceID+']'}},@{n='Value';e={[string]$_.ConfigManagerErrorCode}})
 PowerPlan=[pscustomobject]@{Name='Active plan';Value=((powercfg /getactivescheme) -join '')}
}"#,
    )
}

pub fn gpu_names() -> Vec<String> {
    ps_json("Get-CimInstance Win32_VideoController | Select-Object Name,AdapterRAM,DriverVersion")
        .map(|v| {
            v.iter()
                .map(|g| format!("{} (driver {})", crate::util::jstr(g, "Name"), crate::util::jstr(g, "DriverVersion")))
                .collect()
        })
        .unwrap_or_default()
}

pub fn etw_sessions() -> Result<String, String> {
    run_hidden("logman", &["query", "-ets"])
}
