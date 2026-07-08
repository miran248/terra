#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Material {
    Metal,
    Wood,
    Rope,
    Cloth,
}

impl Material {
    pub const ALL: [Material; 4] = [Material::Metal, Material::Wood, Material::Rope, Material::Cloth];

    pub fn name(&self) -> &'static str {
        match self {
            Material::Metal => "Metal",
            Material::Wood => "Wood",
            Material::Rope => "Rope",
            Material::Cloth => "Cloth",
        }
    }

    pub fn color(&self) -> (f32, f32, f32) {
        match self {
            Material::Metal => (0.6, 0.6, 0.7),
            Material::Wood => (0.5, 0.35, 0.15),
            Material::Rope => (0.8, 0.7, 0.4),
            Material::Cloth => (0.85, 0.85, 0.9),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponKind {
    Knife,
    Spear,
    Pistol,
    Sling,
    Rifle,
}

impl WeaponKind {
    pub const ALL: [WeaponKind; 5] = [
        WeaponKind::Knife,
        WeaponKind::Spear,
        WeaponKind::Pistol,
        WeaponKind::Sling,
        WeaponKind::Rifle,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            WeaponKind::Knife => "Knife",
            WeaponKind::Spear => "Spear",
            WeaponKind::Pistol => "Pistol",
            WeaponKind::Sling => "Sling",
            WeaponKind::Rifle => "Rifle",
        }
    }

    pub fn color(&self) -> (f32, f32, f32) {
        match self {
            WeaponKind::Knife => (0.8, 0.8, 0.85),
            WeaponKind::Spear => (0.7, 0.5, 0.3),
            WeaponKind::Pistol => (0.3, 0.3, 0.35),
            WeaponKind::Sling => (0.75, 0.65, 0.4),
            WeaponKind::Rifle => (0.2, 0.2, 0.25),
        }
    }

    /// damage, range, shots per second, max durability (shots before breaking)
    pub fn stats(&self) -> WeaponStats {
        match self {
            WeaponKind::Knife => WeaponStats { damage: 15.0, range: 60.0, fire_rate: 2.0, durability: 40 },
            WeaponKind::Spear => WeaponStats { damage: 30.0, range: 90.0, fire_rate: 1.5, durability: 60 },
            WeaponKind::Pistol => WeaponStats { damage: 20.0, range: 200.0, fire_rate: 2.5, durability: 80 },
            WeaponKind::Sling => WeaponStats { damage: 12.0, range: 160.0, fire_rate: 1.8, durability: 50 },
            WeaponKind::Rifle => WeaponStats { damage: 40.0, range: 320.0, fire_rate: 3.0, durability: 120 },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WeaponStats {
    pub damage: f32,
    pub range: f32,
    pub fire_rate: f32,
    pub durability: u32,
}

/// A crafting recipe: consume materials to build a weapon.
#[derive(Debug, Clone, Copy)]
pub struct Recipe {
    pub output: WeaponKind,
    pub cost: [(Material, u32); 2],
}

impl Recipe {
    pub const ALL: [Recipe; 5] = [
        Recipe { output: WeaponKind::Knife, cost: [(Material::Metal, 2), (Material::Cloth, 1)] },
        Recipe { output: WeaponKind::Spear, cost: [(Material::Wood, 2), (Material::Metal, 1)] },
        Recipe { output: WeaponKind::Sling, cost: [(Material::Rope, 2), (Material::Cloth, 2)] },
        Recipe { output: WeaponKind::Pistol, cost: [(Material::Metal, 3), (Material::Wood, 1)] },
        Recipe { output: WeaponKind::Rifle, cost: [(Material::Metal, 4), (Material::Wood, 2)] },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_and_weapons_consistent() {
        for w in WeaponKind::ALL {
            assert!(w.stats().durability > 0, "{} needs durability", w.name());
            assert!(w.stats().fire_rate > 0.0);
        }
        for r in Recipe::ALL {
            for (_, n) in r.cost {
                assert!(n > 0, "recipe {} has zero-cost material", r.output.name());
            }
        }
    }
}
