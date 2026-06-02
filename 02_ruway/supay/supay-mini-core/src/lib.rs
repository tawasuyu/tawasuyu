//! `supay-mini-core` — el mundo simulado del raycaster mini de supay,
//! **agnóstico de GUI**.
//!
//! Esto es lo que un frontend NO debe reimplementar: mapa, entidades,
//! reglas del tick (movimiento + colisión sliding, AI de persecución,
//! balística, pickups, fin de partida) y la geometría del raycast
//! (DDA + line-of-sight). El frontend (`supay-app-llimphi` u otro:
//! TUI/web/headless) consume [`World`] + [`cast_ray`] y decide cómo
//! pintarlo — colores, luces, texturas y fog son del renderer.
//!
//! Antes esto vivía dentro de `supay-app-llimphi` (frontend Llimphi),
//! exactamente el patrón que hizo a shuma perder features al cambiar de
//! GUI. Extraído a este core para cumplir la regla #2 del repo.

#![forbid(unsafe_code)]

// =====================================================================
// Mapa hardcoded — 1 = pared, número = material id (1..4), 0 = vacío
// =====================================================================

pub const MAP_W: usize = 16;
pub const MAP_H: usize = 16;

#[rustfmt::skip]
pub const MAP: [u8; MAP_W * MAP_H] = [
    1,1,1,1,1,1,2,2,2,2,2,1,1,1,1,1,
    1,0,0,0,0,1,0,0,0,0,2,0,0,0,0,1,
    1,0,0,0,0,1,0,0,0,0,0,0,0,0,0,1,
    1,0,0,3,0,0,0,0,3,0,2,0,0,0,0,1,
    1,0,0,0,0,1,0,0,0,0,2,1,1,0,1,1,
    1,1,1,0,1,1,0,0,0,0,2,0,0,0,0,1,
    1,0,0,0,0,0,0,0,0,0,0,0,3,0,0,1,
    1,0,0,4,0,0,0,4,0,0,0,0,0,0,0,1,
    1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,
    1,1,0,1,1,0,0,0,0,1,1,0,1,1,0,1,
    1,0,0,0,1,0,0,3,0,1,0,0,0,0,0,1,
    1,0,0,0,1,0,0,0,0,1,0,0,0,0,0,1,
    1,0,0,0,1,1,0,0,1,1,0,0,4,0,0,1,
    1,0,3,0,0,0,0,0,0,0,0,0,0,0,0,1,
    1,0,0,0,0,0,2,2,0,0,0,0,0,0,0,1,
    1,1,1,1,1,1,2,2,1,1,1,1,1,1,1,1,
];

/// Material id de la celda `(x, y)`. Fuera del mapa = pared (`1`).
pub fn tile(x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= MAP_W as i32 || y >= MAP_H as i32 {
        return 1;
    }
    MAP[y as usize * MAP_W + x as usize]
}

// =====================================================================
// Constantes de gameplay y de cámara
// =====================================================================

pub const MOVE_SPEED: f32 = 0.10; // unidades de mapa por tick
pub const STRAFE_SPEED: f32 = 0.08;
pub const TURN_SPEED: f32 = 0.055; // radianes por tick
/// Campo de visión (ángulo total, ~60°). Es parámetro de cámara del
/// mundo — el renderer lo usa para lanzar los rayos.
pub const FOV: f32 = 1.05;

pub const ENEMY_HP: i32 = 100;
pub const ENEMY_SPEED: f32 = 0.045; // u/tick — más lento que el jugador
pub const ENEMY_AGGRO_RANGE: f32 = 8.0; // unidades
pub const ENEMY_MELEE_RANGE: f32 = 0.9;
pub const ENEMY_MELEE_DAMAGE: u32 = 8;
pub const ENEMY_MELEE_CD: u32 = 25; // ticks (~0.7 s entre golpes)
pub const ENEMY_DYING_TICKS: u32 = 14;
pub const BULLET_DAMAGE: i32 = 25;
pub const BULLET_HIT_RADIUS: f32 = 0.35;

pub const BULLET_SPEED: f32 = 0.45; // unidades/tick
pub const BULLET_TTL: u32 = 60; // ticks (~1.7 s)

pub const DECAL_TTL: u32 = 240; // ticks (~7 s)
pub const MAX_DECALS: usize = 32;

pub const FLASH_TTL: u32 = 4; // ticks (~115 ms)
pub const FLASH_STRENGTH_IMPACT: f32 = 3.5;
pub const FLASH_COLOR_IMPACT: (f32, f32, f32) = (1.0, 0.75, 0.30);

pub const AMMO_PICKUP_AMOUNT: u32 = 12;
pub const HEALTH_PICKUP_AMOUNT: u32 = 25;
pub const HEALTH_MAX: u32 = 100;
pub const PICKUP_RADIUS: f32 = 0.55;

// =====================================================================
// Tipos de entidades del mundo
// =====================================================================

/// Estado del input del jugador para el próximo tick. El frontend lo
/// llena desde el teclado; la simulación lo consume en [`World::advance`].
#[derive(Default, Clone, Copy)]
pub struct Input {
    pub forward: bool,
    pub backward: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub turn_left: bool,
    pub turn_right: bool,
}

/// Tipo de sprite del mundo. El color/altura aparente es decisión del
/// renderer (no vive acá).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpriteKind {
    Barrel,
    Pillar,
    Imp,
    Torch,
    Bullet,
    Decal,
    DyingImp,
    Corpse,
    AmmoBox,
    HealthKit,
}

/// Decorado estático del mundo (no se mueve, no pelea).
#[derive(Clone, Copy)]
pub struct Sprite {
    pub x: f32,
    pub y: f32,
    pub kind: SpriteKind,
    /// Multiplicador de tamaño aparente (1.0 = pared completa).
    pub scale: f32,
}

pub fn initial_static_sprites() -> Vec<Sprite> {
    vec![
        Sprite {
            x: 4.5,
            y: 3.5,
            kind: SpriteKind::Barrel,
            scale: 0.5,
        },
        Sprite {
            x: 11.5,
            y: 4.5,
            kind: SpriteKind::Pillar,
            scale: 1.0,
        },
        Sprite {
            x: 6.5,
            y: 9.5,
            kind: SpriteKind::Barrel,
            scale: 0.5,
        },
        Sprite {
            x: 8.5,
            y: 12.5,
            kind: SpriteKind::Torch,
            scale: 0.7,
        },
        Sprite {
            x: 3.5,
            y: 13.5,
            kind: SpriteKind::Torch,
            scale: 0.7,
        },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnemyState {
    Idle,
    Walking,
    Dying(u32),
    Dead,
}

#[derive(Clone, Copy)]
pub struct Enemy {
    pub x: f32,
    pub y: f32,
    pub hp: i32,
    pub state: EnemyState,
    pub attack_cd: u32,
}

pub fn initial_enemies() -> Vec<Enemy> {
    vec![
        Enemy {
            x: 7.5,
            y: 5.5,
            hp: ENEMY_HP,
            state: EnemyState::Idle,
            attack_cd: 0,
        },
        Enemy {
            x: 12.5,
            y: 11.5,
            hp: ENEMY_HP,
            state: EnemyState::Idle,
            attack_cd: 0,
        },
    ]
}

/// Flash temporal (impacto, disparo) con TTL — luz puntual efímera.
#[derive(Clone, Copy)]
pub struct TempLight {
    pub x: f32,
    pub y: f32,
    pub color: (f32, f32, f32),
    pub strength: f32,
    pub ttl: u32,
    pub ttl_max: u32,
}

#[derive(Clone, Copy)]
pub enum PickupKind {
    Ammo,
    Health,
}

#[derive(Clone, Copy)]
pub struct Pickup {
    pub x: f32,
    pub y: f32,
    pub kind: PickupKind,
}

pub fn initial_pickups() -> Vec<Pickup> {
    vec![
        Pickup {
            x: 4.5,
            y: 7.5,
            kind: PickupKind::Ammo,
        },
        Pickup {
            x: 11.5,
            y: 8.5,
            kind: PickupKind::Health,
        },
        Pickup {
            x: 2.5,
            y: 11.5,
            kind: PickupKind::Ammo,
        },
        Pickup {
            x: 13.5,
            y: 14.5,
            kind: PickupKind::Health,
        },
        Pickup {
            x: 6.5,
            y: 14.5,
            kind: PickupKind::Ammo,
        },
    ]
}

/// Proyectil del jugador. Vida finita; muere al chocar pared (deja decal)
/// o enemigo (le pega).
#[derive(Clone, Copy)]
pub struct Bullet {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub ttl: u32,
}

/// Decal de impacto en pared. Estático con TTL.
#[derive(Clone, Copy)]
pub struct Decal {
    pub x: f32,
    pub y: f32,
    pub ttl: u32,
}

// =====================================================================
// Raycast (DDA estilo Lode Vandevenne) — geometría pura del mundo
// =====================================================================

pub struct RayHit {
    /// Distancia perpendicular al plano de cámara (evita fish-eye).
    pub perp_dist: f32,
    pub material: u8,
    /// `true` si la pared es E/W (vertical grid edge); `false` si N/S.
    pub side_ew: bool,
    /// Posición horizontal del hit en la pared, en `[0, 1)`.
    pub wall_x: f32,
}

/// Lanza un rayo desde `(px, py)` en `ray_angle` y devuelve el primer
/// impacto contra pared. Geometría pura: ningún frontend la reimplementa.
pub fn cast_ray(px: f32, py: f32, ray_angle: f32) -> RayHit {
    let (sin, cos) = ray_angle.sin_cos();
    let dir_x = cos;
    let dir_y = sin;

    let delta_x = if dir_x.abs() < 1e-6 {
        1e6
    } else {
        (1.0_f32 / dir_x).abs()
    };
    let delta_y = if dir_y.abs() < 1e-6 {
        1e6
    } else {
        (1.0_f32 / dir_y).abs()
    };

    let mut map_x = px.floor() as i32;
    let mut map_y = py.floor() as i32;

    let (step_x, mut side_x) = if dir_x < 0.0 {
        (-1, (px - map_x as f32) * delta_x)
    } else {
        (1, (map_x as f32 + 1.0 - px) * delta_x)
    };
    let (step_y, mut side_y) = if dir_y < 0.0 {
        (-1, (py - map_y as f32) * delta_y)
    } else {
        (1, (map_y as f32 + 1.0 - py) * delta_y)
    };

    let mut side_ew = false;
    let mut hit = 0_u8;
    for _ in 0..256 {
        if side_x < side_y {
            side_x += delta_x;
            map_x += step_x;
            side_ew = true;
        } else {
            side_y += delta_y;
            map_y += step_y;
            side_ew = false;
        }
        let t = tile(map_x, map_y);
        if t != 0 {
            hit = t;
            break;
        }
    }

    let perp = if side_ew {
        (map_x as f32 - px + (1 - step_x) as f32 * 0.5) / dir_x
    } else {
        (map_y as f32 - py + (1 - step_y) as f32 * 0.5) / dir_y
    };
    let perp = perp.max(0.0001);

    let wall_x_raw = if side_ew {
        py + perp * dir_y
    } else {
        px + perp * dir_x
    };
    let wall_x = wall_x_raw - wall_x_raw.floor();

    RayHit {
        perp_dist: perp,
        material: hit,
        side_ew,
        wall_x,
    }
}

/// Línea de visión libre entre `(ax, ay)` y `(bx, by)` — true = visible.
pub fn has_los(ax: f32, ay: f32, bx: f32, by: f32) -> bool {
    let dx = bx - ax;
    let dy = by - ay;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 0.01 {
        return true;
    }
    let steps = (dist / 0.1).ceil() as i32;
    let inv = 1.0 / steps as f32;
    for i in 1..steps {
        let t = i as f32 * inv;
        let cx = (ax + dx * t).floor() as i32;
        let cy = (ay + dy * t).floor() as i32;
        if tile(cx, cy) != 0 {
            return false;
        }
    }
    true
}

/// `true` si un disco de radio `r` centrado en `(x, y)` toca alguna pared.
pub fn is_blocked(x: f32, y: f32, r: f32) -> bool {
    let x0 = (x - r).floor() as i32;
    let x1 = (x + r).floor() as i32;
    let y0 = (y - r).floor() as i32;
    let y1 = (y + r).floor() as i32;
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            if tile(cx, cy) != 0 {
                return true;
            }
        }
    }
    false
}

// =====================================================================
// World — estado del mundo + reglas del tick
// =====================================================================

/// El mundo simulado: jugador, entidades dinámicas y flags de partida.
/// `tick` lo incrementa el frontend antes de [`advance`](World::advance).
pub struct World {
    pub px: f32,
    pub py: f32,
    pub pa: f32, // ángulo en radianes
    pub input: Input,
    pub tick: u64,
    pub last_hit_material: u8,
    pub health: u32,
    pub ammo: u32,
    pub bullets: Vec<Bullet>,
    pub decals: Vec<Decal>,
    pub static_sprites: Vec<Sprite>,
    pub enemies: Vec<Enemy>,
    pub pickups: Vec<Pickup>,
    pub temp_lights: Vec<TempLight>,
    pub game_over: bool,
    pub victory: bool,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Mundo en estado inicial de partida.
    pub fn new() -> Self {
        World {
            px: 2.5,
            py: 2.5,
            pa: 0.6,
            input: Input::default(),
            tick: 0,
            last_hit_material: 0,
            health: 100,
            ammo: 50,
            bullets: Vec::with_capacity(16),
            decals: Vec::with_capacity(MAX_DECALS),
            static_sprites: initial_static_sprites(),
            enemies: initial_enemies(),
            pickups: initial_pickups(),
            temp_lights: Vec::with_capacity(8),
            game_over: false,
            victory: false,
        }
    }

    /// Reinicia jugador + entidades dinámicas a estado de partida nueva
    /// (conserva los sprites estáticos del nivel).
    pub fn reset(&mut self) {
        self.px = 2.5;
        self.py = 2.5;
        self.pa = 0.6;
        self.input = Input::default();
        self.health = 100;
        self.ammo = 50;
        self.bullets.clear();
        self.decals.clear();
        self.enemies = initial_enemies();
        self.pickups = initial_pickups();
        self.temp_lights.clear();
        self.game_over = false;
        self.victory = false;
        self.last_hit_material = 0;
    }

    /// Dispara una bala desde el jugador, si hay munición y la partida
    /// está viva. Devuelve `true` si efectivamente disparó.
    pub fn fire(&mut self) -> bool {
        if self.game_over || self.victory || self.ammo == 0 {
            return false;
        }
        self.ammo -= 1;
        let (sin, cos) = self.pa.sin_cos();
        self.bullets.push(Bullet {
            x: self.px + cos * 0.25,
            y: self.py + sin * 0.25,
            vx: cos * BULLET_SPEED,
            vy: sin * BULLET_SPEED,
            ttl: BULLET_TTL,
        });
        true
    }

    /// Un tick de simulación: giro/movimiento del jugador, balas,
    /// enemigos, pickups, envejecimiento de decals/flashes y
    /// transiciones de fin de partida.
    pub fn advance(&mut self) {
        // En game_over/victory el mundo se congela; sólo drenan flashes.
        if self.game_over || self.victory {
            self.temp_lights.retain(|tl| tl.ttl > 0);
            for tl in self.temp_lights.iter_mut() {
                tl.ttl = tl.ttl.saturating_sub(1);
            }
            return;
        }
        if self.input.turn_left {
            self.pa -= TURN_SPEED;
        }
        if self.input.turn_right {
            self.pa += TURN_SPEED;
        }
        let two_pi = std::f32::consts::TAU;
        if self.pa < 0.0 {
            self.pa += two_pi;
        } else if self.pa >= two_pi {
            self.pa -= two_pi;
        }

        let (sin, cos) = self.pa.sin_cos();
        let mut dx = 0.0_f32;
        let mut dy = 0.0_f32;
        if self.input.forward {
            dx += cos * MOVE_SPEED;
            dy += sin * MOVE_SPEED;
        }
        if self.input.backward {
            dx -= cos * MOVE_SPEED;
            dy -= sin * MOVE_SPEED;
        }
        if self.input.strafe_left {
            dx += sin * STRAFE_SPEED;
            dy -= cos * STRAFE_SPEED;
        }
        if self.input.strafe_right {
            dx -= sin * STRAFE_SPEED;
            dy += cos * STRAFE_SPEED;
        }

        const RADIUS: f32 = 0.18;
        let new_x = self.px + dx;
        let new_y = self.py + dy;
        if !is_blocked(new_x, self.py, RADIUS) {
            self.px = new_x;
        }
        if !is_blocked(self.px, new_y, RADIUS) {
            self.py = new_y;
        }

        let snap = cast_ray(self.px, self.py, self.pa);
        self.last_hit_material = snap.material;

        self.advance_bullets();
        self.advance_enemies();
        self.consume_pickups();

        self.decals.retain(|d| d.ttl > 0);
        for d in self.decals.iter_mut() {
            d.ttl = d.ttl.saturating_sub(1);
        }
        self.temp_lights.retain(|tl| tl.ttl > 0);
        for tl in self.temp_lights.iter_mut() {
            tl.ttl = tl.ttl.saturating_sub(1);
        }

        if self.health == 0 {
            self.game_over = true;
        } else if self
            .enemies
            .iter()
            .all(|e| matches!(e.state, EnemyState::Dead))
        {
            self.victory = true;
        }
    }

    fn spawn_flash(&mut self, x: f32, y: f32, color: (f32, f32, f32), strength: f32) {
        self.temp_lights.push(TempLight {
            x,
            y,
            color,
            strength,
            ttl: FLASH_TTL,
            ttl_max: FLASH_TTL,
        });
    }

    fn consume_pickups(&mut self) {
        let px = self.px;
        let py = self.py;
        let mut picked: Vec<Pickup> = Vec::new();
        self.pickups.retain(|p| {
            let dx = p.x - px;
            let dy = p.y - py;
            if dx * dx + dy * dy < PICKUP_RADIUS * PICKUP_RADIUS {
                picked.push(*p);
                false
            } else {
                true
            }
        });
        for p in picked {
            match p.kind {
                PickupKind::Ammo => {
                    self.ammo = self.ammo.saturating_add(AMMO_PICKUP_AMOUNT);
                    self.spawn_flash(p.x, p.y, (0.45, 0.85, 0.95), 2.0);
                }
                PickupKind::Health => {
                    self.health = (self.health + HEALTH_PICKUP_AMOUNT).min(HEALTH_MAX);
                    self.spawn_flash(p.x, p.y, (0.30, 0.95, 0.40), 2.2);
                }
            }
        }
    }

    fn advance_enemies(&mut self) {
        let player_x = self.px;
        let player_y = self.py;
        let mut total_damage: u32 = 0;
        for e in self.enemies.iter_mut() {
            e.attack_cd = e.attack_cd.saturating_sub(1);
            match e.state {
                EnemyState::Dead => continue,
                EnemyState::Dying(rem) => {
                    if rem <= 1 {
                        e.state = EnemyState::Dead;
                    } else {
                        e.state = EnemyState::Dying(rem - 1);
                    }
                    continue;
                }
                EnemyState::Idle | EnemyState::Walking => {}
            }
            let dx = player_x - e.x;
            let dy = player_y - e.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > ENEMY_AGGRO_RANGE || !has_los(e.x, e.y, player_x, player_y) {
                e.state = EnemyState::Idle;
                continue;
            }
            e.state = EnemyState::Walking;
            if dist < ENEMY_MELEE_RANGE && e.attack_cd == 0 {
                total_damage = total_damage.saturating_add(ENEMY_MELEE_DAMAGE);
                e.attack_cd = ENEMY_MELEE_CD;
                continue;
            }
            if dist > 0.01 {
                let inv = 1.0 / dist;
                let step_x = dx * inv * ENEMY_SPEED;
                let step_y = dy * inv * ENEMY_SPEED;
                const ER: f32 = 0.18;
                let nx = e.x + step_x;
                if !is_blocked(nx, e.y, ER) {
                    e.x = nx;
                }
                let ny = e.y + step_y;
                if !is_blocked(e.x, ny, ER) {
                    e.y = ny;
                }
            }
        }
        if total_damage > 0 {
            self.health = self.health.saturating_sub(total_damage);
        }
    }

    fn advance_bullets(&mut self) {
        let mut new_decals: Vec<Decal> = Vec::new();
        let mut new_flashes: Vec<(f32, f32)> = Vec::new();
        let mut bullet_hits_enemy: Vec<usize> = Vec::new();
        let mut survivors: Vec<Bullet> = Vec::with_capacity(self.bullets.len());

        for mut b in self.bullets.drain(..) {
            if b.ttl == 0 {
                continue;
            }
            b.ttl -= 1;
            let nx = b.x + b.vx;
            let ny = b.y + b.vy;

            if tile(nx as i32, ny as i32) != 0 {
                new_decals.push(Decal {
                    x: b.x,
                    y: b.y,
                    ttl: DECAL_TTL,
                });
                new_flashes.push((b.x, b.y));
                continue;
            }

            let mut hit_enemy: Option<usize> = None;
            for (i, e) in self.enemies.iter().enumerate() {
                if matches!(e.state, EnemyState::Dead | EnemyState::Dying(_)) {
                    continue;
                }
                let edx = e.x - nx;
                let edy = e.y - ny;
                if edx * edx + edy * edy < BULLET_HIT_RADIUS * BULLET_HIT_RADIUS {
                    hit_enemy = Some(i);
                    break;
                }
            }
            if let Some(i) = hit_enemy {
                bullet_hits_enemy.push(i);
                new_flashes.push((nx, ny));
                continue;
            }

            b.x = nx;
            b.y = ny;
            survivors.push(b);
        }
        self.bullets = survivors;

        for d in new_decals {
            if self.decals.len() >= MAX_DECALS {
                self.decals.remove(0);
            }
            self.decals.push(d);
        }
        for (fx, fy) in new_flashes {
            self.spawn_flash(fx, fy, FLASH_COLOR_IMPACT, FLASH_STRENGTH_IMPACT);
        }
        for i in bullet_hits_enemy {
            if i >= self.enemies.len() {
                continue;
            }
            let e = &mut self.enemies[i];
            if matches!(e.state, EnemyState::Dead | EnemyState::Dying(_)) {
                continue;
            }
            e.hp -= BULLET_DAMAGE;
            if e.hp <= 0 {
                e.state = EnemyState::Dying(ENEMY_DYING_TICKS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_starts_in_a_free_cell() {
        let w = World::new();
        assert_eq!(
            tile(w.px as i32, w.py as i32),
            0,
            "el jugador no arranca dentro de una pared"
        );
        assert!(!w.game_over && !w.victory);
    }

    #[test]
    fn walls_block_movement() {
        let mut w = World::new();
        // Empujar contra la pared oeste (x baja) un buen rato.
        w.input.strafe_left = false;
        w.pa = std::f32::consts::PI; // mirando -x
        w.input.forward = true;
        for _ in 0..200 {
            w.advance();
        }
        assert!(
            w.px > 1.0,
            "la colisión sliding no dejó atravesar la pared: px={}",
            w.px
        );
    }

    #[test]
    fn fire_consumes_ammo_and_spawns_bullet() {
        let mut w = World::new();
        let ammo0 = w.ammo;
        assert!(w.fire());
        assert_eq!(w.ammo, ammo0 - 1);
        assert_eq!(w.bullets.len(), 1);
    }

    #[test]
    fn no_fire_without_ammo() {
        let mut w = World::new();
        w.ammo = 0;
        assert!(!w.fire());
        assert!(w.bullets.is_empty());
    }

    #[test]
    fn cast_ray_hits_a_wall() {
        let w = World::new();
        let hit = cast_ray(w.px, w.py, 0.0);
        assert!(
            hit.material != 0,
            "el rayo debe chocar una pared en el mapa cerrado"
        );
        assert!(hit.perp_dist > 0.0);
    }

    #[test]
    fn killing_all_enemies_is_victory() {
        let mut w = World::new();
        for e in w.enemies.iter_mut() {
            e.state = EnemyState::Dead;
        }
        w.advance();
        assert!(w.victory);
    }

    #[test]
    fn zero_health_is_game_over() {
        let mut w = World::new();
        w.health = 0;
        w.advance();
        assert!(w.game_over);
    }
}
