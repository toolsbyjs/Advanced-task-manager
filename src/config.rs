//! Saved settings: visible columns, rules, profiles, schedules, scripts, dashboards.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Metric {
    SystemCpu,
    SystemMemory,
    CommitCharge,
    DiskActive,
    GpuUsage,
    NetReceiveMBps,
    NetSendMBps,
    ProcessCpu,
    ProcessMemoryMB,
    ProcessHandles,
    ProcessThreads,
    ProcessIoMBps,
    ProcessStarted,
    ProcessExited,
    Temperature,
}

impl Metric {
    pub const ALL: [Metric; 15] = [
        Metric::SystemCpu,
        Metric::SystemMemory,
        Metric::CommitCharge,
        Metric::DiskActive,
        Metric::GpuUsage,
        Metric::NetReceiveMBps,
        Metric::NetSendMBps,
        Metric::ProcessCpu,
        Metric::ProcessMemoryMB,
        Metric::ProcessHandles,
        Metric::ProcessThreads,
        Metric::ProcessIoMBps,
        Metric::ProcessStarted,
        Metric::ProcessExited,
        Metric::Temperature,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Metric::SystemCpu => "System CPU %",
            Metric::SystemMemory => "System memory used %",
            Metric::CommitCharge => "Commit charge % of limit",
            Metric::DiskActive => "Disk active time %",
            Metric::GpuUsage => "GPU usage %",
            Metric::NetReceiveMBps => "Network receive MB/s",
            Metric::NetSendMBps => "Network send MB/s",
            Metric::ProcessCpu => "Process CPU %",
            Metric::ProcessMemoryMB => "Process private memory MB",
            Metric::ProcessHandles => "Process handle count",
            Metric::ProcessThreads => "Process thread count",
            Metric::ProcessIoMBps => "Process disk I/O MB/s",
            Metric::ProcessStarted => "Process starts",
            Metric::ProcessExited => "Process exits",
            Metric::Temperature => "Highest temperature °C",
        }
    }
    pub fn is_process(&self) -> bool {
        matches!(
            self,
            Metric::ProcessCpu
                | Metric::ProcessMemoryMB
                | Metric::ProcessHandles
                | Metric::ProcessThreads
                | Metric::ProcessIoMBps
                | Metric::ProcessStarted
                | Metric::ProcessExited
        )
    }
    pub fn is_event(&self) -> bool {
        matches!(self, Metric::ProcessStarted | Metric::ProcessExited)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Above,
    Below,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl Priority {
    pub const ALL: [Priority; 6] =
        [Priority::Idle, Priority::BelowNormal, Priority::Normal, Priority::AboveNormal, Priority::High, Priority::Realtime];
    pub fn class(&self) -> u32 {
        match self {
            Priority::Idle => 0x40,
            Priority::BelowNormal => 0x4000,
            Priority::Normal => 0x20,
            Priority::AboveNormal => 0x8000,
            Priority::High => 0x80,
            Priority::Realtime => 0x100,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Priority::Idle => "Low (idle)",
            Priority::BelowNormal => "Below normal",
            Priority::Normal => "Normal",
            Priority::AboveNormal => "Above normal",
            Priority::High => "High",
            Priority::Realtime => "Realtime",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    Alert,
    KillProcess,
    SuspendProcess,
    SetPriority(Priority),
    EfficiencyMode,
    RunScript(String),
    StartRecording(u32),
    ApplySystemProfile(String),
}

impl Action {
    pub fn label(&self) -> String {
        match self {
            Action::Alert => "Show an alert".into(),
            Action::KillProcess => "End the process".into(),
            Action::SuspendProcess => "Suspend the process".into(),
            Action::SetPriority(p) => format!("Set priority to {}", p.label()),
            Action::EfficiencyMode => "Put process in efficiency mode".into(),
            Action::RunScript(s) => format!("Run script: {}", s),
            Action::StartRecording(s) => format!("Record performance for {s}s"),
            Action::ApplySystemProfile(p) => format!("Apply system profile '{p}'"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub metric: Metric,
    pub op: Op,
    pub threshold: f64,
    pub duration_s: u32,
    pub target: String,
    pub action: Action,
    pub cooldown_s: u32,
    #[serde(default)]
    pub fired: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcProfile {
    pub id: u64,
    pub pattern: String,
    pub enabled: bool,
    pub priority: Option<Priority>,
    pub affinity: Option<u64>,
    pub efficiency: Option<bool>,
    #[serde(default)]
    pub applied: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SysProfile {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub power_plan: String,
    pub end_processes: Vec<String>,
    pub lower_priority: Vec<String>,
    pub raise_priority: Vec<String>,
    pub sample_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ScheduleKind {
    EveryMinutes(u32),
    DailyAt(u32, u32),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub kind: ScheduleKind,
    pub record_secs: u32,
    pub take_snapshot: bool,
    pub write_report: bool,
    #[serde(default)]
    pub last_run: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Script {
    pub id: u64,
    pub name: String,
    pub shell: String,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Widget {
    CpuChart,
    MemoryChart,
    DiskChart,
    NetworkChart,
    GpuChart,
    CoreGrid,
    TopCpu,
    TopMemory,
    TopDisk,
    Alerts,
    Health,
    Volumes,
    SystemSummary,
    Timeline,
    Sensors,
}

impl Widget {
    pub const ALL: [Widget; 15] = [
        Widget::CpuChart,
        Widget::MemoryChart,
        Widget::DiskChart,
        Widget::NetworkChart,
        Widget::GpuChart,
        Widget::CoreGrid,
        Widget::TopCpu,
        Widget::TopMemory,
        Widget::TopDisk,
        Widget::Alerts,
        Widget::Health,
        Widget::Volumes,
        Widget::SystemSummary,
        Widget::Timeline,
        Widget::Sensors,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Widget::CpuChart => "CPU graph",
            Widget::MemoryChart => "Memory graph",
            Widget::DiskChart => "Disk graph",
            Widget::NetworkChart => "Network graph",
            Widget::GpuChart => "GPU graph",
            Widget::CoreGrid => "CPU cores",
            Widget::TopCpu => "Top CPU users",
            Widget::TopMemory => "Top memory users",
            Widget::TopDisk => "Top disk users",
            Widget::Alerts => "Recent alerts",
            Widget::Health => "Health score",
            Widget::Volumes => "Drives",
            Widget::SystemSummary => "System summary",
            Widget::Timeline => "Latest events",
            Widget::Sensors => "Temperatures",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dashboard {
    pub id: u64,
    pub name: String,
    pub columns: u32,
    pub widgets: Vec<Widget>,
}

/// Look-and-feel and navigation preferences chosen by the person.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiPrefs {
    pub theme: String,
    pub accent: [u8; 3],
    pub zoom: f32,
    pub font_size: f32,
    pub density: String,
    pub chart_fill: bool,
    pub chart_grid: bool,
    pub chart_smooth: bool,
    pub line_width: f32,
    pub rounding: f32,
    pub heat_colors: bool,
    pub row_colors: bool,
    pub status_bar: bool,
    pub nav_compact: bool,
    pub favorites: Vec<String>,
    pub hidden_pages: Vec<String>,
    pub collapsed_groups: Vec<String>,
    pub start_page: String,
    pub toast_secs: u64,
    pub overview_hidden: Vec<String>,
    pub net_bits: bool,
    pub detail_height: f32,
    pub animations: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: "Midnight".into(),
            accent: [56, 189, 248],
            zoom: 1.0,
            font_size: 13.5,
            density: "Comfortable".into(),
            chart_fill: true,
            chart_grid: true,
            chart_smooth: false,
            line_width: 1.6,
            rounding: 10.0,
            heat_colors: true,
            row_colors: true,
            status_bar: true,
            nav_compact: false,
            favorites: vec!["Processes".into(), "Performance".into(), "Health".into()],
            hidden_pages: vec![],
            collapsed_groups: vec![],
            start_page: "Dashboard".into(),
            toast_secs: 6,
            overview_hidden: vec![],
            net_bits: true,
            detail_height: 340.0,
            animations: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub columns: Vec<String>,
    pub sample_ms: u64,
    pub tree_view: bool,
    pub rules: Vec<Rule>,
    pub proc_profiles: Vec<ProcProfile>,
    pub sys_profiles: Vec<SysProfile>,
    pub schedules: Vec<Schedule>,
    pub scripts: Vec<Script>,
    pub dashboards: Vec<Dashboard>,
    pub next_id: u64,
    #[serde(default)]
    pub confirm_actions: bool,
    #[serde(default)]
    pub ui: UiPrefs,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            columns: crate::ui::processes::default_columns(),
            sample_ms: 1000,
            tree_view: false,
            rules: vec![
                Rule {
                    id: 1,
                    name: "CPU stays very busy".into(),
                    enabled: true,
                    metric: Metric::SystemCpu,
                    op: Op::Above,
                    threshold: 90.0,
                    duration_s: 15,
                    target: String::new(),
                    action: Action::Alert,
                    cooldown_s: 300,
                    fired: 0,
                },
                Rule {
                    id: 2,
                    name: "Memory almost full".into(),
                    enabled: true,
                    metric: Metric::SystemMemory,
                    op: Op::Above,
                    threshold: 90.0,
                    duration_s: 10,
                    target: String::new(),
                    action: Action::Alert,
                    cooldown_s: 300,
                    fired: 0,
                },
                Rule {
                    id: 3,
                    name: "One app uses lots of memory".into(),
                    enabled: true,
                    metric: Metric::ProcessMemoryMB,
                    op: Op::Above,
                    threshold: 4096.0,
                    duration_s: 30,
                    target: "*".into(),
                    action: Action::Alert,
                    cooldown_s: 600,
                    fired: 0,
                },
            ],
            proc_profiles: vec![],
            sys_profiles: vec![
                SysProfile {
                    id: 10,
                    name: "Gaming".into(),
                    description: "High performance power plan, background helpers get lower priority.".into(),
                    power_plan: "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".into(),
                    end_processes: vec![],
                    lower_priority: vec!["OneDrive.exe".into(), "SearchIndexer.exe".into(), "msedge.exe".into()],
                    raise_priority: vec![],
                    sample_ms: 2000,
                },
                SysProfile {
                    id: 11,
                    name: "Quiet / battery".into(),
                    description: "Power saver plan and slower sampling to save energy.".into(),
                    power_plan: "a1841308-3541-4fab-bc81-f71556f20b4a".into(),
                    end_processes: vec![],
                    lower_priority: vec![],
                    raise_priority: vec![],
                    sample_ms: 3000,
                },
                SysProfile {
                    id: 12,
                    name: "Balanced".into(),
                    description: "Windows balanced plan and normal sampling.".into(),
                    power_plan: "381b4222-f694-41f0-9685-ff5bb260df2e".into(),
                    end_processes: vec![],
                    lower_priority: vec![],
                    raise_priority: vec![],
                    sample_ms: 1000,
                },
            ],
            schedules: vec![],
            scripts: vec![
                Script {
                    id: 20,
                    name: "Top 5 by memory".into(),
                    shell: "PowerShell".into(),
                    body: "Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 5 Name,Id,@{n='MB';e={[int]($_.WorkingSet64/1MB)}} | Format-Table -AutoSize".into(),
                },
                Script {
                    id: 21,
                    name: "Flush DNS cache".into(),
                    shell: "cmd".into(),
                    body: "ipconfig /flushdns".into(),
                },
            ],
            dashboards: vec![Dashboard {
                id: 30,
                name: "My overview".into(),
                columns: 3,
                widgets: vec![Widget::CpuChart, Widget::MemoryChart, Widget::Health, Widget::TopCpu, Widget::TopMemory, Widget::Alerts],
            }],
            next_id: 100,
            confirm_actions: true,
            ui: UiPrefs::default(),
        }
    }
}

impl Config {
    pub fn path() -> std::path::PathBuf {
        crate::util::data_dir().join("config.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(), s);
        }
    }
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}
