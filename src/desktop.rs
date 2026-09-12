//! Native renderer. The economic world remains authoritative; meshes are derived
//! from its lots, and construction uses the same month-end queue as the web UI.
use crate::{
    city::Zone,
    commands::{Command, SimulationClock},
    sim::{World, DAYS_PER_MONTH, DAYS_PER_YEAR},
};
use macroquad::prelude::*;

pub fn window_config() -> Conf {
    Conf {
        window_title: "Econsim | City workshop".into(),
        window_width: 1440,
        window_height: 900,
        sample_count: 4,
        ..Default::default()
    }
}

fn tint(c: Color, k: f32) -> Color {
    Color::new(c.r * k, c.g * k, c.b * k, c.a)
}
fn zone_color(z: Zone) -> Color {
    match z {
        Zone::Residential => Color::from_hex(0xe4caa3),
        Zone::Business => Color::from_hex(0x79b8cd),
        Zone::School => Color::from_hex(0xe6a26a),
        Zone::Park => Color::from_hex(0x64a877),
        Zone::Road => Color::from_hex(0x505866),
        Zone::Empty => Color::from_hex(0x7d9871),
    }
}
fn block(p: Vec3, size: Vec3, c: Color) {
    draw_cube(p, size, None, c);
    draw_cube_wires(p, size, tint(c, 0.77));
}
fn lot(x: f32, z: f32, zone: Zone, occupants: u16) {
    let color = zone_color(zone);
    block(vec3(x, -0.035, z), vec3(0.94, 0.07, 0.94), color);
    if zone == Zone::Road {
        draw_cube(vec3(x, 0.02, z), vec3(0.98, 0.04, 0.98), None, color);
        return;
    }
    if zone == Zone::Empty {
        return;
    }
    if zone == Zone::Park {
        for (dx, dz) in [(-0.23, -0.2), (0.22, 0.2), (-0.2, 0.25)] {
            block(vec3(x + dx, 0.14, z + dz), vec3(0.055, 0.28, 0.055), BROWN);
            draw_sphere(
                vec3(x + dx, 0.38, z + dz),
                0.19,
                None,
                Color::from_hex(0x357b56),
            );
        }
        block(vec3(x + 0.25, 0.05, z - 0.23), vec3(0.3, 0.1, 0.1), color);
        return;
    }
    let h = match zone {
        Zone::Residential => 0.35 + occupants as f32 / 20.0 * 1.15,
        Zone::Business => 0.5 + occupants as f32 / 6.0 * 2.1,
        _ => 0.48,
    };
    let width = if zone == Zone::School { 0.8 } else { 0.62 };
    block(vec3(x, h / 2.0, z), vec3(width, h, 0.64), color);
    block(
        vec3(x, h + 0.035, z),
        vec3(width + 0.07, 0.07, 0.71),
        Color::from_hex(0x435666),
    );
    for floor in 0..(h / 0.25) as usize {
        for dx in [-0.18, 0.0, 0.18] {
            let glass = Color::from_hex(0x344e64);
            draw_cube(
                vec3(x + dx, 0.16 + floor as f32 * 0.25, z + 0.323),
                vec3(0.09, 0.12, 0.008),
                None,
                glass,
            );
            draw_cube(
                vec3(x + width / 2.0 + 0.003, 0.16 + floor as f32 * 0.25, z + dx),
                vec3(0.008, 0.12, 0.09),
                None,
                glass,
            );
        }
    }
}

/// Ground-plane picking uses the same view/projection matrix as the renderer.
fn pick(matrix: Mat4, mouse: Vec2, screen: Vec2, w: usize, h: usize) -> Option<(usize, usize)> {
    let ndc = vec2(
        2.0 * mouse.x / screen.x - 1.0,
        1.0 - 2.0 * mouse.y / screen.y,
    );
    let inv = matrix.inverse();
    let near = inv.project_point3(vec3(ndc.x, ndc.y, -1.0));
    let far = inv.project_point3(vec3(ndc.x, ndc.y, 1.0));
    let direction = far - near;
    if direction.y.abs() < 1e-6 {
        return None;
    }
    let t = -near.y / direction.y;
    if t < 0.0 {
        return None;
    }
    let p = near + direction * t;
    let x = (p.x + 0.5).floor();
    let y = (p.z + 0.5).floor();
    if x >= 0.0 && y >= 0.0 && x < w as f32 && y < h as f32 {
        Some((x as usize, y as usize))
    } else {
        None
    }
}
fn button(rect: Rect, title: &str, active: bool) -> bool {
    let hover = rect.contains(Vec2::from(mouse_position()));
    draw_rectangle(
        rect.x,
        rect.y,
        rect.w,
        rect.h,
        if active {
            Color::from_hex(0x356d69)
        } else if hover {
            Color::from_hex(0x354859)
        } else {
            Color::from_hex(0x233343)
        },
    );
    draw_text(title, rect.x + 12.0, rect.y + 25.0, 20.0, WHITE);
    hover && is_mouse_button_pressed(MouseButton::Left)
}

fn city_view_size() -> Vec2 {
    vec2(
        (screen_width() - 280.0).max(1.0),
        (screen_height() - 144.0).max(1.0),
    )
}
fn fitted_distance(w: usize, h: usize) -> f32 {
    let size = city_view_size();
    ((w as f32).max(h as f32 * 1.5) * 1.1 * (1.2 / (size.x / size.y)).max(1.0)).min(120.0)
}

pub async fn run(mut world: World, save: Option<String>) {
    world.start_player_session();
    let save_path = save.unwrap_or_else(|| format!("{}/city.econsave", world.cfg.out_dir));
    let resume_root = std::path::PathBuf::from(&world.cfg.out_dir);
    let mut resume_number = 0u64;
    prevent_quit();
    let mut target = vec3(world.city.w as f32 / 2.0, 0.0, world.city.h as f32 / 2.0);
    let mut yaw = 0.75f32;
    let mut pitch = 0.8f32;
    let mut distance = fitted_distance(world.city.w, world.city.h);
    let mut last_mouse = Vec2::from(mouse_position());
    let mut overlay = 0u8; // 0 city, 1 flow/capacity, 2 road access
    let mut tool: Option<Zone> = None;
    let mut paused = true;
    let mut speed = 7.0;
    let mut clock = SimulationClock::new(1);
    let mut steps = 0u32;
    let mut save_requested = false;
    let mut load_requested = false;
    let mut message = "Select a tool, then click a lot. Orders fund at month end; construction takes materials and labor.".to_string();
    loop {
        if is_quit_requested() || is_key_pressed(KeyCode::Escape) {
            match world.save(&save_path) {
                Ok(()) => {
                    if let Err(e) = world.finish() {
                        eprintln!("Could not write final outputs: {e}");
                    }
                    break;
                }
                Err(e) => {
                    paused = true;
                    message = format!("Could not save before exit: {e}. Game remains open.");
                }
            }
        }
        let elapsed = std::time::Duration::from_secs_f32(get_frame_time().max(0.0));
        let dt = elapsed.as_secs_f32().min(0.1);
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
        }
        if is_key_pressed(KeyCode::N) {
            steps = steps.saturating_add(DAYS_PER_MONTH - world.day % DAYS_PER_MONTH);
        }
        let mouse = Vec2::from(mouse_position());
        let over_city = mouse.x > 280.0 && mouse.y > 82.0 && mouse.y < screen_height() - 62.0;
        if over_city {
            distance = (distance - mouse_wheel().1 * 1.4).clamp(6.0, 120.0);
            if is_mouse_button_down(MouseButton::Right) {
                let delta = mouse - last_mouse;
                yaw -= delta.x * 0.006;
                pitch = (pitch + delta.y * 0.006).clamp(0.3, 1.45);
            }
        }
        last_mouse = mouse;
        let forward = vec3(-yaw.sin(), 0.0, -yaw.cos());
        let right = vec3(yaw.cos(), 0.0, -yaw.sin());
        let pan = dt * distance * 0.45;
        if is_key_down(KeyCode::W) {
            target += forward * pan;
        }
        if is_key_down(KeyCode::S) {
            target -= forward * pan;
        }
        if is_key_down(KeyCode::A) {
            target -= right * pan;
        }
        if is_key_down(KeyCode::D) {
            target += right * pan;
        }
        target.x = target.x.clamp(0.0, world.city.w as f32);
        target.z = target.z.clamp(0.0, world.city.h as f32);
        if is_key_pressed(KeyCode::Home) {
            target = vec3(world.city.w as f32 / 2.0, 0.0, world.city.h as f32 / 2.0);
            distance = fitted_distance(world.city.w, world.city.h);
            yaw = 0.75;
            pitch = 0.8;
        }
        if save_requested || is_key_pressed(KeyCode::F5) {
            message = match world.save(&save_path) {
                Ok(()) => format!("Saved to {save_path}"),
                Err(e) => format!("Save failed: {e}"),
            };
            save_requested = false;
        }
        if load_requested || is_key_pressed(KeyCode::F9) {
            // Loading always uses a fresh directory: old run outputs are retained.
            let restored = loop {
                resume_number += 1;
                let path = resume_root.join(format!("resume-{resume_number}"));
                if !path.exists() {
                    break World::load(&save_path, path);
                }
            };
            match restored {
                Ok(mut restored) => {
                    if let Err(e) = world.finish() {
                        eprintln!("Could not flush old outputs: {e}");
                    }
                    restored.start_player_session();
                    world = restored;
                    paused = true;
                    steps = 0;
                    clock = SimulationClock::new(1);
                    message = format!(
                        "Restored day {}. Paused; outputs in {}",
                        world.day, world.cfg.out_dir
                    );
                }
                Err(e) => message = format!("Load failed: {e}"),
            }
            load_requested = false;
        }
        // Pull one tick at a time, so unprocessed debt stays in the clock and
        // pausing can stop real-time catchup immediately.
        let manual_steps = std::mem::take(&mut steps);
        let start = std::time::Instant::now();
        for update in 0..4 {
            let due = clock.advance(
                if update == 0 {
                    elapsed
                } else {
                    std::time::Duration::ZERO
                },
                speed,
                paused,
                if update == 0 { manual_steps } else { 0 },
            );
            if due == 0 {
                break;
            }
            let last_event = world.events.count;
            world.tick_day();
            if world.events.count != last_event {
                if let Some((_, _, msg)) = world
                    .events
                    .recent
                    .iter()
                    .rev()
                    .find(|(d, k, _)| *d == world.day && (k == "construction" || k == "city"))
                {
                    message = msg.clone();
                }
            }
            if world.day % DAYS_PER_YEAR == 0 {
                if let Err(e) = world.finish() {
                    message = format!("Output error: {e}");
                }
            }
            if start.elapsed().as_millis() >= 8 {
                break;
            }
        }
        let view_size = city_view_size();
        let camera = Camera3D {
            aspect: Some(view_size.x / view_size.y),
            // The graphics viewport uses bottom-left coordinates.
            viewport: Some((280, 62, view_size.x as i32, view_size.y as i32)),
            position: target
                + vec3(
                    yaw.sin() * pitch.cos(),
                    pitch.sin(),
                    yaw.cos() * pitch.cos(),
                ) * distance,
            target,
            up: Vec3::Y,
            fovy: 45.0_f32.to_radians(),
            ..Default::default()
        };
        let selected = if over_city {
            pick(
                camera.matrix(),
                mouse - vec2(280.0, 82.0),
                view_size,
                world.city.w,
                world.city.h,
            )
        } else {
            None
        };
        clear_background(Color::from_hex(0xb8ced5));
        set_camera(&camera);
        block(
            vec3(
                world.city.w as f32 / 2.0 - 0.5,
                -0.2,
                world.city.h as f32 / 2.0 - 0.5,
            ),
            vec3(world.city.w as f32 + 0.3, 0.3, world.city.h as f32 + 0.3),
            Color::from_hex(0x536b58),
        );
        for y in 0..world.city.h {
            for x in 0..world.city.w {
                let t = &world.city.tiles[y * world.city.w + x];
                lot(x as f32, y as f32, t.zone, t.occupants);
                if overlay > 0 {
                    let idx=y*world.city.w+x;
                    let color=if overlay==1 {let load=(world.city.traffic.flows[idx] as f32/400.0).min(2.0)/2.0; Color::new(load,1.0-load,0.15,0.8)} else if world.city.road_access(idx as u16) {GREEN} else {RED};
                    if overlay==2 || t.zone==Zone::Road {draw_cube(vec3(x as f32,0.065,y as f32),vec3(0.92,0.04,0.92),None,color);}
                }
            }
        }
        for &(x, y, z) in &world.gov.pending_builds {
            draw_cube_wires(
                vec3(x as f32, 0.15, y as f32),
                vec3(0.95, 0.3, 0.95),
                zone_color(Zone::from_u8(z)),
            );
        }
        for p in &world.construction {
            let (x, y) = world.city.xy(p.tile);
            let progress = p.progress() as f32;
            block(
                vec3(x as f32, 0.035, y as f32),
                vec3(0.85, 0.07, 0.85),
                Color::from_hex(0x9a886d),
            );
            draw_cube_wires(vec3(x as f32, 0.5, y as f32), vec3(0.7, 1.0, 0.7), ORANGE);
            if progress > 0.0 {
                block(
                    vec3(x as f32, progress * 0.5, y as f32),
                    vec3(0.6, progress, 0.6),
                    zone_color(p.zone),
                );
            }
        }
        if let Some((x, y)) = selected {
            let valid = tool.map_or(true, |zone| world.preview_build(x, y, zone).is_ok());
            draw_cube_wires(
                vec3(x as f32, 0.04, y as f32),
                vec3(1.0, 0.1, 1.0),
                if valid { YELLOW } else { RED },
            );
            if is_key_pressed(KeyCode::X) {
                message = match world.apply_command(Command::CancelBuild { x, y }) {
                    Ok(outcome) => outcome.message,
                    Err(e) => format!("Cannot cancel: {e}"),
                };
            } else if let Some(zone) = tool {
                draw_cube_wires(
                    vec3(x as f32, 0.4, y as f32),
                    vec3(0.65, 0.8, 0.65),
                    if valid { YELLOW } else { RED },
                );
                if is_mouse_button_pressed(MouseButton::Left) {
                    message = match world.apply_command(Command::Build { x, y, zone }) {
                        Ok(outcome) => outcome.message,
                        Err(e) => format!("Cannot build: {e}"),
                    };
                }
            }
        }
        set_default_camera();
        let panel = Color::from_hex(0x142432);
        draw_rectangle(0.0, 0.0, screen_width(), 82.0, panel);
        draw_text("ECONSIM", 22.0, 34.0, 28.0, WHITE);
        draw_text("CITY WORKSHOP", 22.0, 59.0, 17.0, Color::from_hex(0x8dbcb3));
        draw_text(
            &format!(
                "Year {}  /  Month {}  /  Day {}",
                world.day / DAYS_PER_YEAR + 1,
                world.day / DAYS_PER_MONTH % 12 + 1,
                world.day % DAYS_PER_MONTH + 1
            ),
            295.0,
            32.0,
            24.0,
            WHITE,
        );
        draw_text(
            &format!(
                "Treasury ${:.0}   |   {} firms   |   {} orders   |   {} projects",
                world.ledger.balance(world.gov.account) as f64 / 100.0,
                world.firms.active_list.len(),
                world.gov.pending_builds.len(),
                world.construction.len()
            ),
            295.0,
            60.0,
            20.0,
            WHITE,
        );
        draw_rectangle(0.0, 82.0, 280.0, screen_height() - 82.0, panel);
        if is_key_pressed(KeyCode::T) { overlay=(overlay+1)%3; }
        draw_text(["T: City", "T: Traffic green <400, red >=800", "T: Access green adjacent road, red none"][overlay as usize], 295.0, 100.0, 16.0, WHITE);
        draw_text("PLAN YOUR CITY", 20.0, 116.0, 21.0, WHITE);
        for (i, (name, z)) in [
            ("Inspect", None),
            ("Homes  /  $1,500", Some(Zone::Residential)),
            ("Business  /  $2,500", Some(Zone::Business)),
            ("School  /  $4,000", Some(Zone::School)),
            ("Park  /  $800", Some(Zone::Park)),
            ("Road  /  $600", Some(Zone::Road)),
            ("Demolish  /  $0", Some(Zone::Empty)),
        ]
        .iter()
        .enumerate()
        {
            if button(
                Rect::new(16.0, 134.0 + i as f32 * 38.0, 248.0, 32.0),
                name,
                tool == *z,
            ) {
                tool = *z;
            }
        }
        if button(
            Rect::new(16.0, 414.0, 118.0, 38.0),
            if paused { "Resume" } else { "Pause" },
            !paused,
        ) {
            paused = !paused;
        }
        if button(Rect::new(142.0, 414.0, 122.0, 38.0), "Month +", false) {
            steps = steps.saturating_add(DAYS_PER_MONTH - world.day % DAYS_PER_MONTH);
        }
        for (i, s) in [1.0, 7.0, 28.0].iter().enumerate() {
            if button(
                Rect::new(16.0 + i as f32 * 84.0, 460.0, 80.0, 38.0),
                &format!("{}d/s", s),
                speed == *s,
            ) {
                speed = *s;
            }
        }
        if button(
            Rect::new(16.0, 506.0, 248.0, 38.0),
            "Cancel queued orders",
            false,
        ) {
            message = match world.apply_command(Command::CancelAllQueuedBuilds) {
                Ok(o) => o.message,
                Err(e) => e.to_string(),
            };
        }
        if button(Rect::new(16.0, 550.0, 118.0, 38.0), "Save / F5", false) {
            save_requested = true;
        }
        if button(Rect::new(142.0, 550.0, 122.0, 38.0), "Load / F9", false) {
            load_requested = true;
        }
        let homes = world.city.occupants_of(Zone::Residential);
        let capacity = world.city.capacity_of(Zone::Residential);
        draw_text(
            &format!("Housing: {homes} / {capacity}"),
            20.0,
            620.0,
            20.0,
            WHITE,
        );
        draw_text(
            "Hover lot + X to cancel work",
            20.0,
            644.0,
            17.0,
            Color::from_hex(0x8dbcb3),
        );
        if let Some((x, y)) = selected {
            let t = &world.city.tiles[y * world.city.w + x];
            let project = world
                .construction
                .iter()
                .find(|p| p.tile == world.city.idx(x, y));
            let lines = if let Some(p) = project {
                vec![
                    format!("Lot {x}, {y}: {} site", p.zone.name()),
                    format!("Work: {:.0}% / {}", p.progress() * 100.0, p.stall.name()),
                    format!(
                        "Materials: {} / {}",
                        p.materials_delivered, p.materials_required
                    ),
                    format!(
                        "Spent: ${:.0} / ${:.0}",
                        p.spent as f64 / 100.0,
                        p.budget as f64 / 100.0
                    ),
                    "Cancel refunds unspent funds.".into(),
                ]
            } else {
                vec![
                    format!("Lot {x}, {y}: {}", t.zone.name()),
                    format!("Occupancy: {} / {}", t.occupants, t.zone.capacity()),
                    format!("Road flow: {} / 400", world.city.traffic.flows[world.city.idx(x,y) as usize]),
                    format!("Road access: {}", world.city.road_access(world.city.idx(x,y))),
                    format!("Land value: ${:.0}", t.land_value as f64 / 100.0),
                ]
            };
            for (i, line) in lines.iter().enumerate() {
                draw_text(line, 20.0, 680.0 + i as f32 * 25.0, 17.0, WHITE);
            }
        }
        draw_rectangle(
            280.0,
            screen_height() - 62.0,
            screen_width() - 280.0,
            62.0,
            panel,
        );
        draw_text(&message, 294.0, screen_height() - 37.0, 18.0, WHITE);
        draw_text("WASD pan  |  Right-drag orbit  |  Wheel zoom  |  Home reset  |  Space pause  |  N month  |  X cancel  |  F5 save / F9 load",294.0,screen_height()-13.0,16.0,Color::from_hex(0x8dbcb3));
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picking_hits_camera_target_and_rejects_outside_city() {
        let matrix = Mat4::perspective_rh_gl(45f32.to_radians(), 1.0, 0.1, 100.0)
            * Mat4::look_at_rh(vec3(4.0, 10.0, 12.0), vec3(4.0, 0.0, 3.0), Vec3::Y);
        assert_eq!(
            pick(matrix, vec2(400.0, 400.0), vec2(800.0, 800.0), 24, 16),
            Some((4, 3))
        );
        assert_eq!(
            pick(matrix, vec2(400.0, 400.0), vec2(800.0, 800.0), 2, 2),
            None
        );
    }
}
