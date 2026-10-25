//! Audio showcase and inspection tool.
//!
//! ```text
//! cargo run -p engine --example audio_demo -- --wav-dir out/audio   # export + level table
//! cargo run -p engine --example audio_demo -- --play                # listen on the device
//! cargo run -p engine --example audio_demo -- --play --section music
//! cargo run -p engine --example audio_demo -- --bench               # startup timings only
//! ```
//!
//! `--wav-dir` writes every sound effect variation and every music stem (plus a full mix per
//! track) as 16-bit WAV and prints peak / RMS / loudness / duration per file.
//! `--play` plays each sound effect in turn, then music ramping intensity 0 → 1 → 0, then a
//! 3D sweep from left to right. `--section sfx|music|spatial` plays just one part.

#[cfg(not(feature = "client"))]
fn main() {
    eprintln!("audio_demo needs the engine's `client` feature");
}

#[cfg(feature = "client")]
fn main() {
    demo::main();
}

#[cfg(feature = "client")]
mod demo {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use engine::audio::dsp::{SAMPLE_RATE, gain_to_db, loudness, peak, rms};
    use engine::audio::music::{MUSIC_SAMPLE_RATE, MusicData, STEM_COUNT, render_track, stem_gains};
    use engine::audio::wav::write_wav;
    use engine::audio::{Audio, MusicTrack, PlayParams, Sfx, generate_all_sfx};
    use engine::glam::{Quat, Vec3};

    pub fn main() {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
        let args: Vec<String> = std::env::args().skip(1).collect();
        let value_of = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };

        let mut did_something = false;
        if args.iter().any(|a| a == "--bench") {
            bench();
            did_something = true;
        }
        if let Some(dir) = value_of("--wav-dir") {
            if let Err(e) = export(Path::new(&dir)) {
                eprintln!("export failed: {e}");
                std::process::exit(1);
            }
            did_something = true;
        }
        if args.iter().any(|a| a == "--play") {
            play(value_of("--section").as_deref());
            did_something = true;
        }
        if !did_something {
            eprintln!("usage: audio_demo [--bench] [--wav-dir <dir>] [--play [--section sfx|music|spatial]]");
        }
    }

    /// Times startup generation the same way `Audio::new` does it (sfx on worker threads here,
    /// one background thread per music track).
    fn bench() {
        let start = Instant::now();
        let music: Vec<_> = MusicTrack::PLAYABLE
            .iter()
            .map(|&t| {
                std::thread::spawn(move || {
                    let s = Instant::now();
                    let data = render_track(t);
                    (t, s.elapsed(), data.map_or(0, |d| d.len()))
                })
            })
            .collect();
        let sfx_start = Instant::now();
        let sfx = generate_all_sfx();
        let sfx_time = sfx_start.elapsed();
        let variants: usize = sfx.iter().map(|v| v.len()).sum();
        println!("sound effects: {} sounds / {variants} variations in {:.0} ms", Sfx::COUNT, ms(sfx_time));
        for handle in music {
            match handle.join() {
                Ok((t, d, len)) => println!(
                    "music {t:?}: {:.1} s loop rendered in {:.0} ms (background thread)",
                    len as f32 / MUSIC_SAMPLE_RATE as f32,
                    ms(d)
                ),
                Err(_) => println!("music thread panicked"),
            }
        }
        println!("everything ready after {:.0} ms", ms(start.elapsed()));
        let start = Instant::now();
        let audio = Audio::new();
        println!(
            "Audio::new() returned after {:.0} ms (enabled: {}; music keeps rendering in the background)",
            ms(start.elapsed()),
            audio.is_enabled()
        );
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    struct Row {
        name: String,
        secs: f32,
        peak_db: f32,
        rms_db: f32,
        /// K-weighted loudness of the loudest 50 ms (what sound effects are normalised on).
        loud_db: f32,
        /// K-weighted loudness of the loudest 400 ms (EBU momentary).
        momentary_db: f32,
    }

    fn analyse(name: String, interleaved: &[f32], channels: usize, rate: u32) -> Row {
        let mono: Vec<f32> = interleaved
            .chunks_exact(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect();
        Row {
            name,
            secs: mono.len() as f32 / rate as f32,
            peak_db: gain_to_db(peak(interleaved)),
            rms_db: gain_to_db(rms(&mono)),
            loud_db: loudness(&mono, rate as f32, 0.05),
            momentary_db: loudness(&mono, rate as f32, 0.4),
        }
    }

    fn export(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut rows = Vec::new();

        let start = Instant::now();
        let all = generate_all_sfx();
        println!("generated {} sound effects in {:.0} ms", Sfx::COUNT, ms(start.elapsed()));
        for (&sfx, variants) in Sfx::ALL.iter().zip(&all) {
            for (v, buf) in variants.iter().enumerate() {
                let name = if v == 0 { sfx.name() } else { format!("{}_v{v}", sfx.name()) };
                write_wav(&dir.join(format!("sfx_{name}.wav")), SAMPLE_RATE, 1, buf)?;
                rows.push(analyse(name, buf, 1, SAMPLE_RATE));
            }
        }
        print_table("SOUND EFFECTS (mono, 44.1 kHz; LU = K-weighted loudness of the loudest window)", &mut rows);

        let mut rows = Vec::new();
        for track in MusicTrack::PLAYABLE {
            let start = Instant::now();
            let Some(data) = render_track(track) else { continue };
            println!(
                "rendered {track:?}: {} BPM, {} bars, {:.2} s loop (loop start {} frames) in {:.0} ms",
                data.bpm,
                data.bars,
                data.len() as f32 / MUSIC_SAMPLE_RATE as f32,
                data.loop_start,
                ms(start.elapsed())
            );
            for stem in &data.stems {
                let flat: Vec<f32> = stem.frames.iter().flatten().copied().collect();
                let name = format!("{track:?}_{}", stem.kind.name());
                write_wav(&dir.join(format!("music_{name}.wav")), MUSIC_SAMPLE_RATE, 2, &flat)?;
                rows.push(analyse(name, &flat, 2, MUSIC_SAMPLE_RATE));
            }
            for (label, intensity) in [("calm", 0.0f32), ("mid", 0.45), ("full", 1.0)] {
                let gains = stem_gains(intensity.max(data.intensity_floor));
                let mix = mix_twice(&data, &gains);
                let name = format!("{track:?}_mix_{label}");
                write_wav(&dir.join(format!("music_{name}.wav")), MUSIC_SAMPLE_RATE, 2, &mix)?;
                rows.push(analyse(name, &mix, 2, MUSIC_SAMPLE_RATE));
            }
        }
        print_table("MUSIC STEMS + MIXES (stereo; mixes play the loop twice to check the seam)", &mut rows);
        println!("wrote WAV files to {}", dir.display());
        Ok(())
    }

    /// Full mix at the given gains as the player outputs it: through the loop point once (the
    /// seam is at `data.len()`), then the loop body a second time.
    fn mix_twice(data: &MusicData, gains: &[f32; STEM_COUNT]) -> Vec<f32> {
        let frames = 2 * data.len() - data.loop_start;
        data.playback(gains, frames).into_iter().flatten().collect()
    }

    fn print_table(title: &str, rows: &mut [Row]) {
        println!("\n{title}");
        println!(
            "{:<28} {:>6} {:>8} {:>8} {:>9} {:>9}",
            "name", "secs", "peak dB", "rms dB", "LU 50ms", "LU 400ms"
        );
        for r in rows.iter() {
            println!(
                "{:<28} {:>6.2} {:>8.1} {:>8.1} {:>9.1} {:>9.1}",
                r.name, r.secs, r.peak_db, r.rms_db, r.loud_db, r.momentary_db
            );
        }
        let loud: Vec<f32> = rows.iter().map(|r| r.loud_db).collect();
        let max = loud.iter().copied().fold(f32::MIN, f32::max);
        let min = loud.iter().copied().fold(f32::MAX, f32::min);
        let peak_max = rows.iter().map(|r| r.peak_db).fold(f32::MIN, f32::max);
        println!("50 ms loudness range {min:.1} .. {max:.1} LU, highest peak {peak_max:.2} dBFS");
    }

    fn wait(audio: &mut Audio, secs: f32) {
        let frame = Duration::from_millis(16);
        let mut left = secs;
        while left > 0.0 {
            std::thread::sleep(frame);
            audio.update(frame.as_secs_f32());
            left -= frame.as_secs_f32();
        }
    }

    fn play(section: Option<&str>) {
        let start = Instant::now();
        let mut audio = Audio::new();
        println!("Audio::new() took {:.0} ms, enabled = {}", ms(start.elapsed()), audio.is_enabled());
        if !audio.is_enabled() {
            println!("no audio device: nothing to play");
            return;
        }
        audio.set_volumes(0.8, 1.0, 0.8);
        audio.set_listener(Vec3::ZERO, Quat::IDENTITY);
        let all = section.is_none();

        if all || section == Some("sfx") {
            let lengths = generate_all_sfx();
            for (&sfx, variants) in Sfx::ALL.iter().zip(&lengths) {
                let secs = variants.first().map_or(0.3, |b| b.len() as f32 / SAMPLE_RATE as f32);
                println!("{:<18} {:.2} s", sfx.name(), secs);
                audio.play_2d(sfx);
                if sfx == Sfx::Ricochet {
                    // A ricochet chain at rising pitch.
                    for k in 1..4 {
                        wait(&mut audio, 0.12);
                        audio.play(sfx, PlayParams::default().with_pitch(1.0 + 0.12 * k as f32));
                    }
                }
                wait(&mut audio, (secs + 0.15).min(1.6));
            }
        }

        if all || section == Some("music") {
            for track in [MusicTrack::Campaign, MusicTrack::Arena] {
                println!("music: {track:?}, intensity 0 -> 1 -> 0");
                audio.set_music_intensity(0.0);
                audio.set_music(track);
                while !audio.music_ready(track) {
                    wait(&mut audio, 0.05);
                }
                let steps = 40;
                for i in 0..=steps * 2 {
                    let x = i as f32 / steps as f32;
                    let intensity = if x <= 1.0 { x } else { 2.0 - x };
                    audio.set_music_intensity(intensity);
                    if i % 8 == 0 {
                        println!("  intensity {intensity:.2}  playhead {:?}", audio.music_position());
                    }
                    wait(&mut audio, 0.25);
                }
            }
            for track in [MusicTrack::Menu, MusicTrack::Results] {
                println!("music: {track:?}");
                audio.set_music(track);
                wait(&mut audio, 8.0);
            }
            audio.set_music(MusicTrack::None);
            wait(&mut audio, 1.6);
        }

        if all || section == Some("spatial") {
            println!("spatial: footsteps and shots sweeping left -> right 6 m in front, then circling");
            for i in 0..=24 {
                let x = -12.0 + i as f32;
                let pos = Vec3::new(x, 0.0, -6.0);
                audio.play_at(if i % 4 == 0 { Sfx::LancetFire } else { Sfx::Footstep }, pos);
                wait(&mut audio, 0.2);
            }
            for i in 0..32 {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                let pos = Vec3::new(a.sin() * 8.0, 0.0, -a.cos() * 8.0);
                audio.play_at(Sfx::HuskAlert, pos);
                wait(&mut audio, 0.25);
            }
            println!("spatial: distance falloff 2 m .. 60 m");
            for d in [2.0f32, 5.0, 10.0, 20.0, 40.0, 59.0] {
                audio.play_at(Sfx::Explosion, Vec3::new(0.0, 0.0, -d));
                wait(&mut audio, 1.0);
            }
            wait(&mut audio, 1.5);
        }
    }
}
