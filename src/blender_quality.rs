#[derive(Debug, Clone, Copy)]
pub enum BlenderOutputKind {
    Scene,
    Animation,
}

#[derive(Debug, Clone)]
pub struct BlenderQualityBrief {
    pub kind: BlenderOutputKind,
    pub style: String,
    pub has_reference_image: bool,
    pub review_checklist: Vec<&'static str>,
}

impl BlenderQualityBrief {
    pub fn for_scene(style: &str, has_reference_image: bool) -> Self {
        Self {
            kind: BlenderOutputKind::Scene,
            style: style.to_string(),
            has_reference_image,
            review_checklist: vec![
                "clear focal subject and readable composition",
                "camera motion feels deliberate instead of random",
                "lighting and materials feel premium and product-ready",
                "branding and UI cues remain recognizable",
                "final render is easy to extend inside the Rust editing pipeline",
            ],
        }
    }

    pub fn for_animation(style: &str) -> Self {
        Self {
            kind: BlenderOutputKind::Animation,
            style: style.to_string(),
            has_reference_image: false,
            review_checklist: vec![
                "main idea is visually obvious in the first seconds",
                "timing is smooth enough for later clipping and export",
                "camera, typography, and motion support clarity",
                "contrast is strong enough for delivery-page previews",
                "result can be combined cleanly with voice, music, and FFmpeg finishing",
            ],
        }
    }

    pub fn guidance_block(&self) -> String {
        let kind_line = match self.kind {
            BlenderOutputKind::Scene => {
                "Target a polished commercial 3D scene with readable staging, premium lighting, and purposeful camera motion."
            }
            BlenderOutputKind::Animation => {
                "Target a polished motion sequence with clean pacing, strong readability, and edit-friendly timing."
            }
        };

        let reference_line = if self.has_reference_image {
            "Preserve the brand cues, layout hierarchy, and visual identity suggested by the reference image."
        } else {
            "Create strong visual hierarchy even without a supplied brand reference."
        };

        format!(
            "{kind_line} Style direction: {}. {reference_line} Review against: {}.",
            self.style,
            self.review_checklist.join(", ")
        )
    }
}

pub fn enrich_scene_prompt(prompt: &str, style: &str, has_reference_image: bool) -> String {
    let brief = BlenderQualityBrief::for_scene(style, has_reference_image);
    format!(
        "{prompt}\n\nCreative direction: {}",
        brief.guidance_block()
    )
}

pub fn enrich_animation_description(description: &str, quality: &str) -> String {
    let brief = BlenderQualityBrief::for_animation(quality);
    format!(
        "{description}\n\nCreative direction: {}",
        brief.guidance_block()
    )
}
