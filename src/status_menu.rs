use crate::disk::{format_decimal_gigabytes, DiskObservation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusReading<T> {
    Unavailable,
    Current(T),
    Stale(T),
}

impl StatusReading<u8> {
    pub fn title(self, label: &str) -> String {
        metric_title(label, self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusMenuItemId {
    CpuAndRam,
    Disk,
    OpenSystemUsage,
    ReviewSpace,
    OpenPreferences,
    OpenHistory,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusMenuGroupId {
    Readings,
    Investigation,
    Application,
    Exit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusMenuItem {
    id: StatusMenuItemId,
    title: String,
    enabled: bool,
}

impl StatusMenuItem {
    pub const fn id(&self) -> StatusMenuItemId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusMenuGroup {
    id: StatusMenuGroupId,
    items: Vec<StatusMenuItem>,
}

impl StatusMenuGroup {
    pub const fn id(&self) -> StatusMenuGroupId {
        self.id
    }

    pub fn items(&self) -> &[StatusMenuItem] {
        &self.items
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusMenuPresentation {
    groups: Vec<StatusMenuGroup>,
}

impl StatusMenuPresentation {
    pub fn new(
        cpu: StatusReading<u8>,
        ram: StatusReading<u8>,
        disk: StatusReading<DiskObservation>,
        mole_integration_enabled: bool,
    ) -> Self {
        let mut readings = vec![StatusMenuItem {
            id: StatusMenuItemId::CpuAndRam,
            title: format!(
                "{} · {}",
                metric_title("CPU", cpu),
                metric_title("RAM", ram)
            ),
            enabled: false,
        }];
        if mole_integration_enabled {
            readings.push(StatusMenuItem {
                id: StatusMenuItemId::Disk,
                title: disk_title(disk),
                enabled: false,
            });
        }
        Self {
            groups: vec![
                StatusMenuGroup {
                    id: StatusMenuGroupId::Readings,
                    items: readings,
                },
                StatusMenuGroup {
                    id: StatusMenuGroupId::Investigation,
                    items: vec![
                        action(StatusMenuItemId::OpenSystemUsage, "Uso do sistema…", true),
                        action(
                            StatusMenuItemId::ReviewSpace,
                            "Revisar espaço…",
                            mole_integration_enabled,
                        ),
                    ],
                },
                StatusMenuGroup {
                    id: StatusMenuGroupId::Application,
                    items: vec![
                        action(StatusMenuItemId::OpenPreferences, "Preferências…", true),
                        action(StatusMenuItemId::OpenHistory, "Histórico…", true),
                    ],
                },
                StatusMenuGroup {
                    id: StatusMenuGroupId::Exit,
                    items: vec![action(StatusMenuItemId::Quit, "Sair", true)],
                },
            ],
        }
    }

    pub fn item(&self, id: StatusMenuItemId) -> Option<&StatusMenuItem> {
        self.groups
            .iter()
            .flat_map(|group| group.items.iter())
            .find(|item| item.id == id)
    }

    pub fn groups(&self) -> &[StatusMenuGroup] {
        &self.groups
    }
}

fn action(id: StatusMenuItemId, title: &str, enabled: bool) -> StatusMenuItem {
    StatusMenuItem {
        id,
        title: title.to_owned(),
        enabled,
    }
}

fn disk_title(reading: StatusReading<DiskObservation>) -> String {
    match reading {
        StatusReading::Unavailable => "Disco — leitura indisponível".to_owned(),
        StatusReading::Current(observation) => format!(
            "Disco {} disponíveis — leitura atual",
            format_decimal_gigabytes(observation.available_bytes())
        ),
        StatusReading::Stale(observation) => format!(
            "Disco {} disponíveis — leitura antiga",
            format_decimal_gigabytes(observation.available_bytes())
        ),
    }
}

fn metric_title(name: &str, reading: StatusReading<u8>) -> String {
    match reading {
        StatusReading::Unavailable => format!("{name} — leitura indisponível"),
        StatusReading::Current(percent) => format!("{name} {percent}% — leitura atual"),
        StatusReading::Stale(percent) => format!("{name} {percent}% — leitura antiga"),
    }
}
