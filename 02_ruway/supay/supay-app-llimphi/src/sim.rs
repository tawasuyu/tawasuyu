use super::*;

// =====================================================================
// Lógica del tick — movimiento + colisión simple cell-based
// =====================================================================

pub(crate) fn advance(m: &mut Model) {
    // Si está en game_over o victory, el mundo se congela — sólo
    // envejecen flashes/decals para que el efecto siga drenando.
    if m.game_over || m.victory {
        m.temp_lights.retain(|tl| tl.ttl > 0);
        for tl in m.temp_lights.iter_mut() {
            tl.ttl = tl.ttl.saturating_sub(1);
        }
        return;
    }
    if m.input.turn_left {
        m.pa -= TURN_SPEED;
    }
    if m.input.turn_right {
        m.pa += TURN_SPEED;
    }
    // mantener [0, 2π)
    let two_pi = std::f32::consts::TAU;
    if m.pa < 0.0 {
        m.pa += two_pi;
    } else if m.pa >= two_pi {
        m.pa -= two_pi;
    }

    let (sin, cos) = m.pa.sin_cos();
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    if m.input.forward {
        dx += cos * MOVE_SPEED;
        dy += sin * MOVE_SPEED;
    }
    if m.input.backward {
        dx -= cos * MOVE_SPEED;
        dy -= sin * MOVE_SPEED;
    }
    if m.input.strafe_left {
        dx += sin * STRAFE_SPEED;
        dy -= cos * STRAFE_SPEED;
    }
    if m.input.strafe_right {
        dx -= sin * STRAFE_SPEED;
        dy += cos * STRAFE_SPEED;
    }

    // Movimiento por eje con colisión separada (sliding contra paredes).
    const RADIUS: f32 = 0.18;
    let new_x = m.px + dx;
    let new_y = m.py + dy;
    if !is_blocked(new_x, m.py, RADIUS) {
        m.px = new_x;
    }
    if !is_blocked(m.px, new_y, RADIUS) {
        m.py = new_y;
    }

    // Snapshot del material apuntado al centro de la pantalla
    // (rayo recto) — útil para HUD/debug.
    let snap = cast_ray(m.px, m.py, m.pa);
    m.last_hit_material = snap.material;

    // Avance de bullets + colisión vs pared + colisión vs enemy.
    advance_bullets(m);
    // AI y movimiento de enemies.
    advance_enemies(m);
    // Pickups que el jugador toca.
    consume_pickups(m);
    // Envejecimiento de decals + temp_lights.
    m.decals.retain(|d| d.ttl > 0);
    for d in m.decals.iter_mut() {
        d.ttl = d.ttl.saturating_sub(1);
    }
    m.temp_lights.retain(|tl| tl.ttl > 0);
    for tl in m.temp_lights.iter_mut() {
        tl.ttl = tl.ttl.saturating_sub(1);
    }
    // Transiciones de fin de partida — chequeadas al final del tick.
    if m.health == 0 {
        m.game_over = true;
    } else if m.enemies.iter().all(|e| matches!(e.state, EnemyState::Dead)) {
        m.victory = true;
    }
}

pub(crate) fn consume_pickups(m: &mut Model) {
    // Cobramos los pickups que el jugador toca este tick. Usamos
    // `drain_filter` manual con un swap_remove backwards-safe.
    let px = m.px;
    let py = m.py;
    let mut picked: Vec<Pickup> = Vec::new();
    m.pickups.retain(|p| {
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
                m.ammo = m.ammo.saturating_add(AMMO_PICKUP_AMOUNT);
                spawn_flash(m, p.x, p.y, (0.45, 0.85, 0.95), 2.0);
            }
            PickupKind::Health => {
                m.health = (m.health + HEALTH_PICKUP_AMOUNT).min(HEALTH_MAX);
                spawn_flash(m, p.x, p.y, (0.30, 0.95, 0.40), 2.2);
            }
        }
    }
}

pub(crate) fn spawn_flash(m: &mut Model, x: f32, y: f32, color: (f32, f32, f32), strength: f32) {
    m.temp_lights.push(TempLight {
        x,
        y,
        color,
        strength,
        ttl: FLASH_TTL,
        ttl_max: FLASH_TTL,
    });
}

/// AI por enemy:
/// - Si está muerto/dying: solo decrementa countdown si dying.
/// - Si está vivo: chequea LOS al jugador; si la hay y dist <
///   `ENEMY_AGGRO_RANGE`, persigue. Si dist < `ENEMY_MELEE_RANGE` y
///   `attack_cd == 0`, pega al jugador y resetea cooldown.
pub(crate) fn advance_enemies(m: &mut Model) {
    let player_x = m.px;
    let player_y = m.py;
    let mut total_damage: u32 = 0;
    for e in m.enemies.iter_mut() {
        // Cooldown del ataque siempre decrementa.
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
        // Melee: golpea cuando está pegado.
        if dist < ENEMY_MELEE_RANGE && e.attack_cd == 0 {
            total_damage = total_damage.saturating_add(ENEMY_MELEE_DAMAGE);
            e.attack_cd = ENEMY_MELEE_CD;
            continue;
        }
        // Persecución: vector unitario × speed, colisión cell-based.
        if dist > 0.01 {
            let inv = 1.0 / dist;
            let step_x = dx * inv * ENEMY_SPEED;
            let step_y = dy * inv * ENEMY_SPEED;
            // Eje X primero, eje Y después — sliding contra paredes.
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
        m.health = m.health.saturating_sub(total_damage);
    }
}

/// Avanza cada bullet. Tres maneras de morir:
/// 1. Choca pared → decal + flash.
/// 2. Choca enemy alive (dist < `BULLET_HIT_RADIUS`) → enemy.hp -=
///    BULLET_DAMAGE + flash; sin decal.
/// 3. TTL agotado → muerte silenciosa.
pub(crate) fn advance_bullets(m: &mut Model) {
    let mut new_decals: Vec<Decal> = Vec::new();
    let mut new_flashes: Vec<(f32, f32)> = Vec::new();
    let mut bullet_hits_enemy: Vec<usize> = Vec::new(); // idx enemy
    let mut survivors: Vec<Bullet> = Vec::with_capacity(m.bullets.len());

    for mut b in m.bullets.drain(..) {
        if b.ttl == 0 {
            continue;
        }
        b.ttl -= 1;
        let nx = b.x + b.vx;
        let ny = b.y + b.vy;

        // 1. Pared.
        if tile(nx as i32, ny as i32) != 0 {
            new_decals.push(Decal {
                x: b.x,
                y: b.y,
                ttl: DECAL_TTL,
            });
            new_flashes.push((b.x, b.y));
            continue;
        }

        // 2. Enemy alive — chequea contra todos.
        let mut hit_enemy: Option<usize> = None;
        for (i, e) in m.enemies.iter().enumerate() {
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
    m.bullets = survivors;

    for d in new_decals {
        if m.decals.len() >= MAX_DECALS {
            m.decals.remove(0);
        }
        m.decals.push(d);
    }
    for (fx, fy) in new_flashes {
        spawn_flash(m, fx, fy, FLASH_COLOR_IMPACT, FLASH_STRENGTH_IMPACT);
    }
    // Aplicar daño a enemies golpeados (puede ocurrir varias veces
    // contra el mismo enemy si varias balas lo tocan en el mismo tick).
    for i in bullet_hits_enemy {
        if i >= m.enemies.len() {
            continue;
        }
        let e = &mut m.enemies[i];
        if matches!(e.state, EnemyState::Dead | EnemyState::Dying(_)) {
            continue;
        }
        e.hp -= BULLET_DAMAGE;
        if e.hp <= 0 {
            e.state = EnemyState::Dying(ENEMY_DYING_TICKS);
        }
    }
}

pub(crate) fn is_blocked(x: f32, y: f32, r: f32) -> bool {
    // Bounding box AABB del jugador contra celdas.
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
