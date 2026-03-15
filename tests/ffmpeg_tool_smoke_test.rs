// FFmpeg Tool Smoke Test Suite — Option C: Data-driven, 1 row per tool
//
// Covers all ~280 testable FFmpeg tools + 3 Option A agent pipeline tools.
// Skipped tools: hardware-only (nvenc/vaapi/qsv), API-key-dependent (pexels,
//   generate_*, view_*, analyze_image, analyze_pexels_thumbnail),
//   external-file-required (lut3d, dnn models),
//   and agent-control tools (submit_final_answer, set_chat_title).
//
// Run: cargo test --test ffmpeg_tool_smoke_test -- --nocapture

use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use video_editor::agent::tool_executor::execute_tool_claude;

// ─── Shared fixture paths (created once before all cases) ────────────────────
const V1: &str = "outputs/smoke_fx_v1.mp4"; // 5-sec 640×360 video + audio
const V2: &str = "outputs/smoke_fx_v2.mp4"; // duplicate for two-input tools
const A1: &str = "outputs/smoke_fx_a1.wav"; // 5-sec mono sine-wave audio

fn create_fixtures() {
    std::fs::create_dir_all("outputs").ok();

    // Primary video fixture
    if !Path::new(V1).exists() {
        let s = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "lavfi", "-i", "smptebars=size=640x360:rate=25",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100",
                "-t", "5",
                "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28",
                "-c:a", "aac", "-b:a", "64k",
                V1,
            ])
            .status().expect("ffmpeg not found");
        assert!(s.success(), "Failed to create V1 fixture");
    }

    // Secondary video fixture (same content — used for two-input tools)
    if !Path::new(V2).exists() {
        std::fs::copy(V1, V2).expect("Failed to copy V1 → V2");
    }

    // Audio-only fixture
    if !Path::new(A1).exists() {
        let s = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100",
                "-t", "5",
                A1,
            ])
            .status().expect("ffmpeg not found");
        assert!(s.success(), "Failed to create A1 fixture");
    }
}

// ─── Test case definition ─────────────────────────────────────────────────────

struct Case {
    tool: &'static str,
    args: Value,
    /// Expected output file path. None = analysis tool (no file expected).
    output: Option<String>,
}

/// Standard single-video → single-video case (mp4 output, all params default).
fn v(tool: &'static str) -> Case {
    let out = format!("outputs/t_{}.mp4", tool);
    Case {
        tool,
        args: json!({"input_file": V1, "output_file": out}),
        output: Some(out),
    }
}

/// Standard single-video → single-video case with extra args.
fn vx(tool: &'static str, extra: Value) -> Case {
    let out = format!("outputs/t_{}.mp4", tool);
    let mut args = json!({"input_file": V1, "output_file": out});
    merge_json(&mut args, extra);
    Case { tool, args, output: Some(out) }
}

/// Video → custom-extension output.
fn ve(tool: &'static str, ext: &str, extra: Value) -> Case {
    let out = format!("outputs/t_{}.{}", tool, ext);
    let mut args = json!({"input_file": V1, "output_file": out});
    merge_json(&mut args, extra);
    Case { tool, args, output: Some(out) }
}

/// Audio-in → audio-out (uses A1 fixture).
fn a(tool: &'static str, ext: &str) -> Case {
    let out = format!("outputs/t_{}.{}", tool, ext);
    Case {
        tool,
        args: json!({"input_file": A1, "output_file": out}),
        output: Some(out),
    }
}

/// Audio-in → audio-out with extra args.
fn ax(tool: &'static str, ext: &str, extra: Value) -> Case {
    let out = format!("outputs/t_{}.{}", tool, ext);
    let mut args = json!({"input_file": A1, "output_file": out});
    merge_json(&mut args, extra);
    Case { tool, args, output: Some(out) }
}

/// Two-video → single output.
fn v2(tool: &'static str, extra: Value) -> Case {
    let out = format!("outputs/t_{}.mp4", tool);
    let mut args = json!({"input_file": V1, "output_file": out});
    merge_json(&mut args, extra);
    Case { tool, args, output: Some(out) }
}

/// Analysis tool — no output file, just assert no ❌.
fn analysis(tool: &'static str, extra: Value) -> Case {
    let mut args = json!({"input_file": V1});
    merge_json(&mut args, extra);
    Case { tool, args, output: None }
}

/// Analysis on audio.
fn audio_analysis(tool: &'static str, extra: Value) -> Case {
    let mut args = json!({"input_file": A1});
    merge_json(&mut args, extra);
    Case { tool, args, output: None }
}

/// Generator — no input file.
fn gen(tool: &'static str, ext: &str, extra: Value) -> Case {
    let out = format!("outputs/t_{}.{}", tool, ext);
    let mut args = json!({"output_file": out});
    merge_json(&mut args, extra);
    Case { tool, args, output: Some(out) }
}

fn merge_json(base: &mut Value, extra: Value) {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
}

// ─── All test cases ───────────────────────────────────────────────────────────

fn all_cases() -> Vec<Case> {
    vec![
        // ── CORE EDITING ──────────────────────────────────────────────────────
        vx("trim_video",        json!({"start_seconds": 0.5, "end_seconds": 3.0})),
        // merge_videos tested in COMPOSITION section below (two inputs required)
        analysis("analyze_video", json!({})),
        // split_video outputs pattern files — no single output to check
        Case {
            tool: "split_video",
            args: json!({"input_file": V1, "output_prefix": "outputs/t_split_video", "segment_duration": 2.5}),
            output: None,
        },
        vx("rotate_video",      json!({"degrees": 90})),
        vx("flip_video",        json!({"direction": "horizontal"})),
        vx("resize_video",      json!({"width": 320, "height": 180})),
        vx("scale_video",       json!({"width": 320, "height": 180})),
        vx("crop_video",        json!({"x": 0, "y": 0, "width": 320, "height": 180})),
        vx("adjust_speed",      json!({"speed_factor": 1.5})),
        vx("stabilize_video",   json!({"shakiness": 5})),
        vx("deinterlace_video", json!({})),
        v("reverse_video"),
        vx("loop_video",        json!({"count": 2})),
        vx("convert_framerate", json!({"fps": 30.0})),
        vx("decimate_frames",   json!({"decimation": 2})),
        vx("pad_video_time",    json!({"pad_duration": 1.0, "pad_position": "end"})),
        // segment_video outputs pattern files — no single output to check
        Case {
            tool: "segment_video",
            args: json!({"input_file": V1, "output_file": "outputs/t_segment_video_%03d.mp4", "segment_time": 2}),
            output: None,
        },
        vx("deshake_video",     json!({})),
        v("deinterlace_yadif"),

        // stabilize_video_2pass (vidstab — 2-pass)
        vx("stabilize_video_2pass", json!({"shakiness": 5, "accuracy": 15, "smoothing": 10, "zoom": 0.0})),

        // ── VISUAL EFFECTS ────────────────────────────────────────────────────
        vx("add_text_overlay",  json!({"text": "Smoke", "x": 50, "y": 50, "font_size": 24, "color": "white"})),
        vx("adjust_color",      json!({"brightness": 0.1, "contrast": 1.1, "saturation": 1.2})),
        vx("add_animated_text", json!({"text": "Test", "animation_type": "fade_in", "duration": 2.0})),
        vx("apply_filter_chain",json!({"filters": [{"name": "brightness", "value": 0.1}]})),
        vx("picture_in_picture",json!({"main_video": V1, "pip_video": V2, "pip_width": 160, "pip_height": 90, "pip_x": 10, "pip_y": 10})),
        vx("chroma_key",        json!({"background_file": V2, "key_color": "0x00FF00", "similarity": 0.3})),
        vx("split_screen",      json!({"video1": V1, "video2": V2, "orientation": "horizontal"})),
        vx("add_overlay",       json!({"overlay_file": V2, "x_position": 0, "y_position": 0})),
        vx("add_transition",    json!({"input_file1": V1, "input_file2": V2, "transition_type": "fade", "duration": 0.5})),
        vx("add_vignette",      json!({})),
        vx("draw_box",          json!({"x": 10, "y": 10, "width": 100, "height": 100, "color": "red"})),
        v("apply_negate"),
        v("apply_monochrome"),
        vx("add_film_grain",    json!({"all_strength": 15, "flags": ""})),
        vx("apply_rotate_angle",json!({"angle": 45.0, "fillcolor": "black"})),
        vx("posterize_video",   json!({"level": 4})),
        vx("solarize_video",    json!({"threshold": 128.0})),
        vx("apply_pixelize",    json!({"block_width": 8, "block_height": 8})),
        v("select_thumbnail_frame"),
        vx("luma_key",          json!({"threshold": 0.1, "tolerance": 0.1})),
        vx("apply_color_key",   json!({"color": "green", "similarity": 0.1, "blend": 0.0})),
        vx("apply_fade_video",  json!({"transition": "in", "duration": 0.5, "color": "black"})),
        vx("apply_edgedetect",  json!({"low": 0.05, "high": 0.15})),
        vx("chromatic_aberration", json!({"red_shift": 2, "blue_shift": -2})),

        // ── BLUR & SPATIAL FILTERS ────────────────────────────────────────────
        vx("apply_gaussian_blur",json!({"sigma": 2.0})),
        vx("apply_box_blur",    json!({"size_x": 3, "size_y": 3})),
        vx("apply_smart_blur",  json!({"luma_radius": 3.0, "luma_strength": -1.0, "luma_threshold": 0})),
        vx("apply_bilateral",   json!({"sigma_spatial": 5.0, "sigma_range": 0.1})),
        vx("unsharp_mask",      json!({})),
        vx("apply_unsharp_mask",json!({"luma_amount": 1.5})),
        vx("apply_cas",         json!({"strength": 0.5})),
        vx("apply_median_filter",json!({"radius": 2})),
        vx("apply_median_spatial",json!({"radius": 1})),
        vx("apply_spp",         json!({"quality": 4})),
        vx("apply_pp",          json!({"subfilters": "hb/vb/dr/al"})),
        vx("apply_deblock",     json!({"block_size": 8, "alpha": 0.1, "beta": 0.05})),

        // ── COLOR GRADING ─────────────────────────────────────────────────────
        vx("adjust_hue",        json!({"hue": 30.0})),
        vx("color_balance",     json!({})),
        vx("normalize_video",   json!({})),
        vx("adjust_color_temperature", json!({"temperature": 6500.0})),
        vx("adjust_vibrance",   json!({"intensity": 0.5})),
        vx("adjust_curves",     json!({"preset": "vintage"})),
        vx("adjust_levels",     json!({})),
        vx("split_tone",        json!({})),
        vx("convert_colorspace",json!({})),
        vx("apply_tonemap",     json!({})),
        vx("adjust_hue_saturation", json!({"hue": 10.0, "saturation": 0.5, "lightness": 0.0})),
        vx("apply_colormatrix", json!({"matrix": "bt601"})),
        vx("apply_chromashift", json!({"cbh": 2, "cbv": 0, "crh": -2, "crv": 0})),
        vx("apply_colorchannelmixer", json!({"red_red": 1.0, "red_green": 0.0, "red_blue": 0.0, "green_red": 0.0, "green_green": 1.0, "green_blue": 0.0, "blue_red": 0.0, "blue_green": 0.0, "blue_blue": 1.0})),
        vx("apply_colorlevels", json!({"rimin": 0.0, "rimax": 1.0, "gimax": 1.0, "bimax": 1.0})),
        vx("apply_colorhold",   json!({"color": "blue", "similarity": 0.1, "blend": 0.0})),
        vx("apply_pseudocolor", json!({"index": 1})),
        vx("apply_greyedge",    json!({"difford": 0})),
        vx("colorize_video",    json!({"color": "orange", "strength": 0.3})),
        vx("adjust_exposure",   json!({"exposure": 0.5, "black_point": 0.0})),
        vx("apply_hsvhold",     json!({"hue": 120.0, "white": 0.5, "black": 0.0, "similarity": 0.1, "blend": 0.0})),
        vx("apply_hsv_key",     json!({"hue": 120.0, "saturation": 0.5, "value": 0.5})),
        vx("apply_lut_rgb",     json!({"r_expr": "val", "g_expr": "val", "b_expr": "val"})),
        vx("apply_lut_yuv",     json!({"y_expr": "val", "u_expr": "val", "v_expr": "val"})),
        vx("apply_shuffleplanes",json!({"map0": 0, "map1": 1, "map2": 2, "map3": 3})),
        vx("fix_banding",       json!({"radius": 16, "strength": 1.5})),
        vx("apply_histogram_eq",json!({"strength": 0.5})),
        vx("apply_clahe",       json!({"clip_limit": 2.0, "nb_tiles_x": 8, "nb_tiles_y": 8})),
        vx("apply_geq",         json!({"red_expr": "r(X,Y)", "green_expr": "g(X,Y)", "blue_expr": "b(X,Y)"})),
        vx("apply_convolution", json!({"matrix": "0 0 0 0 1 0 0 0 0", "rdiv": 1.0, "bias": 0.0, "planes": 7})),

        // ── DENOISING ─────────────────────────────────────────────────────────
        vx("denoise_video",     json!({})),
        vx("reduce_noise",      json!({})),
        vx("denoise_hqdn3d",    json!({"luma_spatial": 4.0, "luma_temporal": 3.0, "chroma_spatial": 3.0, "chroma_temporal": 2.5})),
        vx("apply_nlmeans_video",json!({"strength": 1.0, "patch_size": 7, "research_size": 15})),
        vx("apply_atadenoise",  json!({})),
        vx("apply_vaguedenoiser",json!({"threshold": 2.0, "nsteps": 6, "percent": 85.0})),
        vx("apply_fftdnoiz",    json!({"tn": 10.0})),
        vx("apply_chromanr",    json!({"thres": 30.0})),
        vx("apply_amplify",     json!({"radius": 2, "factor": 2.0, "threshold": 10.0, "planes": 7})),

        // ── MOTION / TEMPORAL ─────────────────────────────────────────────────
        vx("zoompan",           json!({"zoom_factor": 1.5, "duration_frames": 125, "fps": 25.0})),
        vx("minterpolate",      json!({"fps_target": 50.0})),
        vx("temporal_median",   json!({"radius": 1, "percentile": 50.0})),
        vx("blend_frames",      json!({"blend_file": V2, "weight": 0.5})),
        vx("temporal_blend",    json!({"frames": 3})),
        vx("motion_interpolate",json!({"fps": 50.0})),
        vx("zoom_pan",          json!({"zoom_factor": 1.5, "duration": 125, "fps": 25.0})),
        vx("apply_mestimate",   json!({"method": "epzs"})),
        vx("apply_freezeframes",json!({"first": 0, "last": 0, "replace": 24})),
        vx("apply_framestep",   json!({"step": 2})),
        vx("apply_random_frames",json!({"frames": 5})),
        vx("select_frames",     json!({"selection_expr": "not(mod(n,2))"})),
        vx("apply_lagfun",      json!({"decay": 0.95})),
        vx("tile_frames",       json!({"layout": "2x2", "margin": 2, "padding": 2})),
        vx("apply_telecine",    json!({"pattern": "23"})),
        vx("apply_pullup",      json!({})),
        vx("apply_tinterlace",  json!({"mode": "interlacex2", "flags": "low_pass_filter"})),
        vx("apply_interlace",   json!({"scan": "tff"})),
        vx("apply_weave",       json!({"first_field": "top"})),
        vx("apply_fieldorder",  json!({"order": "tff"})),
        vx("apply_setsar",      json!({"sar": "1:1"})),
        vx("apply_setdar",      json!({"aspect_ratio": "16:9"})),

        // ── EDGE / SHAPE FILTERS ─────────────────────────────────────────────
        vx("apply_sobel",       json!({"planes": 7, "scale": 1.0, "delta": 0.0})),
        vx("apply_roberts",     json!({"planes": 7, "scale": 1.0, "delta": 0.0})),
        vx("apply_prewitt",     json!({"planes": 7, "scale": 1.0, "delta": 0.0})),
        vx("apply_kirsch",      json!({"planes": 7, "scale": 1.0, "delta": 0.0})),
        vx("apply_dilation",    json!({})),
        vx("apply_erosion",     json!({})),
        vx("apply_threshold",   json!({"planes": 15})),
        vx("apply_maskedclamp", json!({"undershoot": 10, "overshoot": 10, "planes": 15})),
        vx("apply_video_limiter",json!({"threshold": 30.0, "planes": 15})),
        vx("apply_bilateral",   json!({"sigma_spatial": 5.0, "sigma_range": 0.1})),
        Case {
            tool: "apply_midequalizer",
            args: json!({"input_file": V1, "secondary_file": V2, "output_file": "outputs/t_apply_midequalizer.mp4", "planes": 15}),
            output: Some("outputs/t_apply_midequalizer.mp4".into()),
        },

        // ── GEOMETRY ──────────────────────────────────────────────────────────
        vx("correct_perspective",json!({"x0": "0", "y0": "0", "x1": "640", "y1": "0", "x2": "640", "y2": "360", "x3": "0", "y3": "360", "interpolation": "linear"})),
        vx("correct_lens",      json!({"focal_length": 35.0})),
        vx("apply_shear",       json!({"shear_x": 0.1, "shear_y": 0.0})),
        vx("correct_lens_simple",json!({"distortion": -0.1, "focal_length": 35.0})),
        vx("correct_perspective_linear", json!({"x0": 0, "y0": 0, "x1": 640, "y1": 0, "x2": 640, "y2": 360, "x3": 0, "y3": 360})),
        vx("pad_video",         json!({"width": 1280, "height": 720, "x": 320, "y": 180, "color": "black"})),
        vx("apply_fillborders", json!({"left": 5, "right": 5, "top": 5, "bottom": 5, "mode": "smear"})),
        vx("apply_swaprect",    json!({"w": 100, "h": 100, "x1": 0, "y1": 0, "x2": 200, "y2": 100})),
        vx("apply_stereo3d",    json!({"in_format": "2l", "out_format": "anb"})),
        // apply_hsvhold already covered in COLOR GRADING section above

        // ── COMPOSITION ───────────────────────────────────────────────────────
        vx("blend_videos",      json!({"input_file1": V1, "input_file2": V2})),
        vx("stack_videos",      json!({"input_file1": V1, "input_file2": V2, "direction": "vertical"})),
        Case {
            tool: "apply_vstack",
            args: json!({"input_file": V1, "secondary_file": V2, "output_file": "outputs/t_apply_vstack.mp4"}),
            output: Some("outputs/t_apply_vstack.mp4".into()),
        },
        Case {
            tool: "apply_hstack",
            args: json!({"input_file": V1, "secondary_file": V2, "output_file": "outputs/t_apply_hstack.mp4"}),
            output: Some("outputs/t_apply_hstack.mp4".into()),
        },
        Case {
            tool: "grid_stack_videos",
            args: json!({"input_files": [V1, V2], "output_file": "outputs/t_grid_stack_videos.mp4", "cols": 2, "rows": 1}),
            output: Some("outputs/t_grid_stack_videos.mp4".into()),
        },
        Case {
            tool: "merge_videos",
            args: json!({"input_files": [V1, V2], "output_file": "outputs/t_merge_videos.mp4"}),
            output: Some("outputs/t_merge_videos.mp4".into()),
        },
        vx("draw_grid",         json!({"width": 80, "height": 45, "color": "white@0.5"})),
        Case {
            tool: "apply_maskedmerge",
            args: json!({"input_file": V1, "overlay_file": V2, "mask_file": V2, "output_file": "outputs/t_apply_maskedmerge.mp4"}),
            output: Some("outputs/t_apply_maskedmerge.mp4".into()),
        },
        Case {
            tool: "displace_video",
            args: json!({"input_file": V1, "xmap_file": V2, "ymap_file": V2, "output_file": "outputs/t_displace_video.mp4"}),
            output: Some("outputs/t_displace_video.mp4".into()),
        },

        // ── ALPHA CHANNEL ─────────────────────────────────────────────────────
        ve("extract_alpha_channel", "mp4", json!({})),
        vx("merge_alpha_channel",   json!({"alpha_file": V2})),

        // ── SCOPE / ANALYSIS VISUALIZATION ───────────────────────────────────
        vx("analyze_vectorscope",json!({"output_file": "outputs/t_analyze_vectorscope.mp4", "mode": "color2"})),
        vx("analyze_waveform",  json!({"output_file": "outputs/t_analyze_waveform.mp4", "mode": "column"})),
        vx("draw_signal_graph", json!({"signal": "src", "width": 640, "height": 240})),
        vx("apply_datascope",   json!({"size": "320x240", "x": 0, "y": 0, "mode": "mono"})),
        vx("apply_mestimate",   json!({"method": "epzs", "mb_size": 16, "search_param": 7})),

        // ── ENCODING / EXPORT ─────────────────────────────────────────────────
        ve("convert_format",    "mkv",  json!({"format": "mkv"})),
        vx("compress_video",    json!({"quality": 28})),
        vx("export_custom_quality", json!({"quality": 28, "width": 640, "height": 360})),
        vx("export_for_platform",   json!({"platform": "youtube"})),
        ve("create_thumbnail",  "jpg",  json!({"time_seconds": 2.0, "width": 320, "height": 180})),
        ve("create_thumbnail_hd","jpg", json!({"time_seconds": 2.0, "width": 640, "height": 360})),
        ve("encode_vp9",        "webm", json!({"crf": 33, "speed": 4})),
        ve("encode_av1",        "mp4",  json!({"crf": 35, "speed": 8})),
        ve("encode_hevc",       "mp4",  json!({"bitrate": "800k", "preset": "ultrafast"})),
        ve("encode_prores",     "mov",  json!({"profile": 0})),
        ve("encode_gif",        "gif",  json!({"fps": 10, "width": 320, "height": 180})),
        ve("encode_webm",       "webm", json!({"bitrate": "500k"})),
        ve("encode_hdr10",      "mp4",  json!({"bitrate": "2M"})),
        ve("encode_dnxhd",      "mov",  json!({"bitrate": "185M", "frame_size": "1920x1080"})),
        ve("encode_prores",     "mov",  json!({"profile": 2})),
        vx("convert_pixel_format", json!({"pixel_format": "yuv420p"})),
        ve("encode_opus",       "ogg",  json!({"bitrate": "128k"})),
        // segment_video (second, different output pattern)
        Case {
            tool: "segment_video",
            args: json!({"input_file": V1, "output_file": "outputs/t_segment2_%03d.mp4", "segment_time": 2}),
            output: None,
        },
        Case {
            tool: "optimize_gif_palette",
            args: json!({"input_file": V1, "output_file": "outputs/t_optimize_gif_palette.gif"}),
            output: Some("outputs/t_optimize_gif_palette.gif".into()),
        },

        // ── AUDIO PROCESSING ─────────────────────────────────────────────────
        ve("extract_audio",     "aac",  json!({})),
        vx("add_audio",         json!({"audio_file": A1, "audio_volume": 0.8})),
        vx("adjust_volume",     json!({"volume": 0.7})),
        vx("fade_audio",        json!({"fade_in_duration": 0.5, "fade_out_duration": 0.5})),
        vx("compress_audio",    json!({})),
        vx("normalize_audio",   json!({})),
        vx("equalize_audio",    json!({})),
        vx("gate_audio",        json!({})),
        vx("denoise_audio",     json!({})),
        vx("normalize_loudness",json!({"i": -16.0, "lra": 11.0, "tp": -1.5})),
        vx("dynamic_audio_normalize", json!({})),
        vx("resample_audio",    json!({"sample_rate": 48000})),
        vx("trim_audio",        json!({"start": 0.5, "duration": 3.0})),
        // denoise_speech_rnn — requires external .rnnn model file — skip
        vx("reduce_sibilance",  json!({"split_hz": 8500.0, "threshold": 0.3})),
        vx("audio_limiter",     json!({"limit_db": -1.0, "attack_ms": 1.0, "release_ms": 50.0})),
        vx("parametric_eq",     json!({})),
        vx("audio_compand",     json!({})),

        // Filter effects
        ax("filter_highpass",   "wav", json!({"frequency": 200.0})),
        ax("filter_lowpass",    "wav", json!({"frequency": 8000.0})),
        ax("adjust_bass",       "wav", json!({"gain": 3.0})),
        ax("adjust_treble",     "wav", json!({"gain": 3.0})),
        ax("add_audio_delay",   "wav", json!({"delay_ms": 500.0, "decay": 0.4})),
        ax("add_phaser",        "wav", json!({})),
        ax("remove_clicks",     "wav", json!({})),
        ax("restore_clipping",  "wav", json!({})),
        ax("remove_silence",    "wav", json!({"noise_db": -60.0})),
        ax("denoise_audio",     "wav", json!({})),
        ax("denoise_audio_nlm", "wav", json!({"strength": 0.05})),
        ax("denoise_audio_fft", "wav", json!({"noise_floor": -60.0})),
        ax("add_echo",          "wav", json!({"in_gain": 0.8, "out_gain": 0.9, "delays": 1000.0, "decays": 0.5})),
        ax("add_chorus",        "wav", json!({"in_gain": 0.4, "out_gain": 0.4})),
        ax("add_vibrato",       "wav", json!({"frequency": 5.0, "depth": 0.5})),
        ax("add_tremolo",       "wav", json!({"frequency": 5.0, "depth": 0.5})),
        ax("add_flanger",       "wav", json!({})),
        ax("apply_crystalizer", "wav", json!({"intensity": 0.1})),
        ax("multiband_compress","wav", json!({})),
        ax("noise_gate",        "wav", json!({"threshold": -60.0})),
        ax("compress_dynamics", "wav", json!({"threshold": -20.0, "ratio": 3.0, "attack": 5.0, "release": 50.0})),
        ax("widen_stereo",      "wav", json!({"width": 1.5})),
        ax("normalize_speech",  "wav", json!({"peak": 0.95})),
        ax("remove_silence_simple","wav", json!({"threshold": -60.0, "min_duration": 0.1})),
        ax("soft_clip_audio",   "wav", json!({"threshold": 0.8})),
        ax("apply_atempo",      "wav", json!({"tempo": 1.25})),
        ax("apply_asetrate",    "wav", json!({"sample_rate": 22050})),
        ax("resample_audio",    "wav", json!({"sample_rate": 22050})),
        ax("apply_dc_shift",    "wav", json!({"shift": 0.0, "limitergain": 0.0})),
        ax("loop_audio",        "wav", json!({"loops": 2})),
        ax("apply_apad",        "wav", json!({"packet_size": 4096, "pad_dur": 1.0})),
        ax("apply_asetnsamples","wav", json!({"n": 1024})),
        ax("apply_asubcut",     "wav", json!({"cutoff": 20.0})),
        ax("apply_asupercut",   "wav", json!({"cutoff": 22000.0})),
        ax("apply_allpass_filter","wav",json!({"frequency": 1000.0})),
        ax("apply_highshelf",   "wav", json!({"frequency": 4000.0, "gain": 3.0})),
        ax("apply_lowshelf",    "wav", json!({"frequency": 400.0, "gain": 3.0})),
        ax("apply_single_eq_band","wav",json!({"type": "peaking", "frequency": 1000.0, "width": 1.0, "gain": 3.0})),
        ax("filter_bandpass",   "wav", json!({"frequency": 1000.0, "width": 500.0})),
        ax("filter_bandreject", "wav", json!({"frequency": 1000.0, "width": 500.0})),
        ax("boost_sub_bass",    "wav", json!({"dry": 1.0, "wet": 1.0, "freq": 100.0, "decay": 0.0})),
        ax("apply_biquad",      "wav", json!({"type": "lowpass", "frequency": 1000.0, "width": 1.0})),
        ax("apply_stereotools", "wav", json!({})),
        ax("apply_stereo_widen","wav", json!({"width": 1.5})),
        ax("adjust_stereo_width","wav",json!({"width": 1.5})),
        ax("apply_crossfeed",   "wav", json!({"strength": 0.5})),
        ax("apply_extrastereo", "wav", json!({"multiplier": 2.0, "clipping": true})),
        ax("apply_acrusher",    "wav", json!({"samples": 64, "bits": 12, "mix": 0.5})),
        ax("apply_earwax",      "wav", json!({})),
        ax("apply_haas",        "wav", json!({"wet": 1.0, "dry": 0.0, "delay": 40.0})),
        ax("apply_aemphasis",   "wav", json!({"type": "50fm"})),
        ax("apply_audio_contrast","wav",json!({"contrast": 33.0})),
        ax("decode_hdcd",       "wav", json!({})),
        ax("shift_audio_frequency","wav",json!({"shift": 100.0})),
        ax("apply_audio_pulsator","wav",json!({"mode": "sine", "rate": 2.0})),
        ax("apply_compensation_delay","wav",json!({"mm": 0, "cm": 0, "m": 1, "dry": 0.0, "wet": 1.0, "temp": 20.0})),
        ax("apply_audio_expression","wav",json!({"exprs": "val"})),
        ax("convert_audio_format","wav",json!({"sample_fmts": "s16", "sample_rates": "44100"})),
        ax("apply_firequalizer","wav",json!({"gain_entry": "entry(0,0)|entry(22050,0)"})),
        ax("apply_super_equalizer","wav",json!({"gains": "1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0 1.0"})),

        // Two-audio inputs
        Case {
            tool: "apply_cross_correlate",
            args: json!({"input_file": A1, "secondary_file": A1, "output_file": "outputs/t_apply_cross_correlate.wav", "size": 256, "algo": "best"}),
            output: Some("outputs/t_apply_cross_correlate.wav".into()),
        },
        Case {
            tool: "apply_audio_multiply",
            args: json!({"input_file": A1, "secondary_file": A1, "output_file": "outputs/t_apply_audio_multiply.wav"}),
            output: Some("outputs/t_apply_audio_multiply.wav".into()),
        },
        Case {
            tool: "blend_audio_streams",
            args: json!({"input_file": A1, "secondary_file": A1, "output_file": "outputs/t_blend_audio_streams.wav", "mix_level": 0.5, "mix_level2": 0.5}),
            output: Some("outputs/t_blend_audio_streams.wav".into()),
        },
        Case {
            tool: "merge_audio_inputs",
            args: json!({"input_files": [A1, A1], "output_file": "outputs/t_merge_audio_inputs.wav"}),
            output: Some("outputs/t_merge_audio_inputs.wav".into()),
        },
        Case {
            tool: "reverse_audio",
            args: json!({"input_file": A1, "output_file": "outputs/t_reverse_audio.wav"}),
            output: Some("outputs/t_reverse_audio.wav".into()),
        },
        // apply_audio_iir covered below in audio section

        // Surround
        ax("apply_surround_upmix","wav",json!({"chl_out": "stereo", "chl_in": "stereo"})),
        ax("mix_audio_channels","wav",json!({"channels": 2, "weights": "1 1"})),
        ax("split_audio_channels","wav",json!({})),
        ax("map_audio_channels","wav",json!({})),
        // render_binaural — requires external HRIR files — skip
        ax("apply_audio_iir",   "wav", json!({"zeros": "1+0i", "poles": "0.9+0i", "gains": "1"})),

        // ── AUDIO VISUALIZATION ───────────────────────────────────────────────
        Case {
            tool: "visualize_cqt",
            args: json!({"input_file": A1, "output_file": "outputs/t_visualize_cqt.mp4"}),
            output: Some("outputs/t_visualize_cqt.mp4".into()),
        },
        Case {
            tool: "visualize_frequencies",
            args: json!({"input_file": A1, "output_file": "outputs/t_visualize_frequencies.mp4", "mode": "line"}),
            output: Some("outputs/t_visualize_frequencies.mp4".into()),
        },
        vx("generate_waveform_video", json!({"input_file": A1, "output_file": "outputs/t_generate_waveform_video.mp4", "rate": 25})),

        // ── ANALYSIS — VIDEO ─────────────────────────────────────────────────
        analysis("analyze_video",       json!({})),
        analysis("get_video_duration",  json!({})),
        analysis("detect_scene_changes",json!({"threshold": 40.0})),
        analysis("detect_silence",      json!({"noise_tolerance_db": -60.0, "min_duration_sec": 0.1})),
        analysis("detect_black_frames", json!({"black_min_duration": 0.05, "picture_black_ratio_th": 0.98})),
        analysis("detect_interlace_type",json!({})),
        analysis("detect_frozen_frames",json!({"noise_db": -60.0, "duration": 0.1})),
        analysis("measure_siti",        json!({"log_file": ""})),
        analysis("measure_video_entropy",json!({})),
        analysis("analyze_video_signal",json!({"filter": "signalstats"})),
        analysis("detect_volume_levels",json!({})),

        // ── ANALYSIS — AUDIO ─────────────────────────────────────────────────
        analysis("measure_loudness",    json!({})),
        analysis("measure_lufs",        json!({"target_lufs": -16.0})),
        audio_analysis("measure_dynamic_range", json!({})),
        audio_analysis("measure_silence",json!({"noise_tolerance_db": -60.0, "min_duration_sec": 0.1})),
        audio_analysis("measure_audio_spectrum", json!({"scale": "log"})),
        analysis("analyze_audio_stats", json!({"stat_file": ""})),

        // ── QUALITY COMPARISON (uses V1 as both distorted + reference) ────────
        Case {
            tool: "compare_ssim",
            args: json!({"input_file": V1, "reference_file": V2, "stats_file": ""}),
            output: None,
        },
        Case {
            tool: "compare_psnr",
            args: json!({"input_file": V1, "reference_file": V2, "stats_file": ""}),
            output: None,
        },

        // ── SPECIALIZED / NICHE ──────────────────────────────────────────────
        vx("convert_360_video", json!({"input_format": "equirect", "output_format": "equirect"})),
        vx("apply_stereo3d",    json!({"in_format": "2l", "out_format": "anb"})),
        vx("apply_greyedge",    json!({"difford": 0})),
        vx("apply_fspp",        json!({"quality": 4})),
        vx("despill_video",     json!({"type": "green"})),
        vx("luma_key",          json!({"threshold": 0.1, "tolerance": 0.1})),
        vx("apply_color_key",   json!({"color": "green", "similarity": 0.1, "blend": 0.0})),
        vx("apply_xfade_transition", json!({"secondary_file": V2, "transition": "fade", "duration": 0.5, "offset": 2.0})),

        // ── GENERATOR (no input) ──────────────────────────────────────────────
        gen("create_test_pattern", "mp4", json!({"width": 640, "height": 360, "duration": 3.0, "pattern": "smptebars", "framerate": 25.0})),
        gen("create_blank_video",  "mp4", json!({"width": 640, "height": 360, "duration": 3.0, "color": "black", "fps": 25})),

        // ── OPTION A — AI AGENT PIPELINE TOOLS ───────────────────────────────
        // analyze_pexels_thumbnail: skipped — requires live Pexels URL + Gemini API key
        Case {
            tool: "generate_video_queries",
            args: json!({"topic": "smoke test topic", "style": "cinematic", "count": 3}),
            output: None,
        },
        Case {
            tool: "verify_clip_quality_tool",
            args: json!({"input_file": V1}),
            output: None,
        },
        Case {
            tool: "run_video_qa",
            args: json!({"input_file": V1}),
            output: None,
        },
    ]
}

// ─── Merge-videos case needs special handling (input_file is overridden) ─────

// ─── Main smoke test ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_all_ffmpeg_tools() {
    create_fixtures();
    std::fs::create_dir_all("outputs").ok();

    let cases = all_cases();
    let total = cases.len();

    let failures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let sem = Arc::new(Semaphore::new(6)); // 6 parallel FFmpeg processes
    let mut join_set: JoinSet<()> = JoinSet::new();

    for case in cases {
        let failures = failures.clone();
        let sem = sem.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let result = execute_tool_claude(case.tool, &case.args).await;

            if result.starts_with('❌') {
                let snippet = &result[..result.len().min(180)];
                failures.lock().unwrap().push(
                    format!("  FAIL  {} — {}", case.tool, snippet)
                );
            } else if let Some(path) = &case.output {
                if !Path::new(path.as_str()).exists() {
                    failures.lock().unwrap().push(
                        format!("  FAIL  {} — output file not created: {}", case.tool, path)
                    );
                }
            }
        });
    }

    while join_set.join_next().await.is_some() {}

    let failures = Arc::try_unwrap(failures).unwrap().into_inner().unwrap();
    let passed = total - failures.len();
    println!("\n✅ {}/{} tools passed", passed, total);

    if !failures.is_empty() {
        println!("❌ {} tools failed:", failures.len());
        for f in &failures {
            println!("{}", f);
        }
        panic!("{}/{} FFmpeg tool smoke tests failed", failures.len(), total);
    }
}
