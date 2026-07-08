#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Upgrade {
    TurretDamage,
    TurretSpeed,
    TurretRange,
    ProjectileSpeed,
    Multishot,
    Piercing,
    Bounces,
    Splits,
    ScrapValue,
    RareScrap,
    SpawnRate,
    WallHp,
    MagnetRange,
}

impl Upgrade {
    pub const ALL: [Upgrade; 13] = [
        Upgrade::TurretDamage,
        Upgrade::TurretSpeed,
        Upgrade::TurretRange,
        Upgrade::ProjectileSpeed,
        Upgrade::Multishot,
        Upgrade::Piercing,
        Upgrade::Bounces,
        Upgrade::Splits,
        Upgrade::ScrapValue,
        Upgrade::RareScrap,
        Upgrade::SpawnRate,
        Upgrade::WallHp,
        Upgrade::MagnetRange,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Upgrade::TurretDamage => "Turret Damage",
            Upgrade::TurretSpeed => "Turret Fire Rate",
            Upgrade::TurretRange => "Turret Range",
            Upgrade::ProjectileSpeed => "Proj Speed",
            Upgrade::Multishot => "Multishot",
            Upgrade::Piercing => "Piercing",
            Upgrade::Bounces => "Bounces",
            Upgrade::Splits => "Splits",
            Upgrade::ScrapValue => "Scrap Value",
            Upgrade::RareScrap => "Rare Scrap",
            Upgrade::SpawnRate => "Spawn Rate",
            Upgrade::WallHp => "Wall HP",
            Upgrade::MagnetRange => "Magnet Range",
        }
    }

    pub fn cost(&self, level: u32) -> u32 {
        let base = match self {
            Upgrade::TurretDamage => 10,
            Upgrade::TurretSpeed => 15,
            Upgrade::TurretRange => 20,
            Upgrade::ProjectileSpeed => 12,
            Upgrade::Multishot => 30,
            Upgrade::Piercing => 35,
            Upgrade::Bounces => 40,
            Upgrade::Splits => 50,
            Upgrade::ScrapValue => 20,
            Upgrade::RareScrap => 25,
            Upgrade::SpawnRate => 18,
            Upgrade::WallHp => 25,
            Upgrade::MagnetRange => 12,
        };
        (base as f32 * 1.4f32.powi(level as i32)) as u32
    }

    pub fn value(&self, level: u32) -> f32 {
        match self {
            Upgrade::TurretDamage => 10.0 * (1.0 + 0.5 * level as f32),
            Upgrade::TurretSpeed => 1.25 * (1.0 + 0.5 * level as f32),
            Upgrade::TurretRange => 50.0 * (1.0 + 0.5 * level as f32),
            Upgrade::ProjectileSpeed => 130.0 * (1.0 + 0.5 * level as f32),
            Upgrade::Multishot => (1.0 + level as f32).min(5.0),
            Upgrade::Piercing => (0.0 + level as f32).min(5.0),
            Upgrade::Bounces => (0.0 + level as f32).min(5.0),
            Upgrade::Splits => (0.0 + level as f32).min(5.0),
            Upgrade::ScrapValue => 1.0 + level as f32,
            Upgrade::RareScrap => (0.0 + level as f32 * 0.05).min(0.5),
            Upgrade::SpawnRate => 0.667 * (1.0 + 0.5 * level as f32),
            Upgrade::WallHp => 500.0 * (1.0 + 0.5 * level as f32),
            Upgrade::MagnetRange => 40.0 * (1.0 + 0.5 * level as f32),
        }
    }

    pub fn format_value(&self, val: f32) -> String {
        match self {
            Upgrade::TurretDamage => format!("{:.0}", val),
            Upgrade::TurretSpeed => format!("{:.2}/s", val),
            Upgrade::TurretRange => format!("{:.0}m", val),
            Upgrade::ProjectileSpeed => format!("{:.0}m/s", val),
            Upgrade::Multishot => format!("x{:.0}", val),
            Upgrade::Piercing => format!("+{:.0}", val),
            Upgrade::Bounces => format!("+{:.0}", val),
            Upgrade::Splits => format!("+{:.0}", val),
            Upgrade::ScrapValue => format!("x{:.0}", val),
            Upgrade::RareScrap => format!("{:.0}%", val * 100.0),
            Upgrade::SpawnRate => format!("{:.2}/s", val),
            Upgrade::WallHp => format!("{:.0}", val),
            Upgrade::MagnetRange => format!("{:.0}m", val),
        }
    }
}
