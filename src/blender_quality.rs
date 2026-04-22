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
    pub camera_language: &'static str,
    pub motion_language: &'static str,
    pub scene_construction: &'static str,
    pub delivery_goal: &'static str,
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
            camera_language: "Use a controlled product-demo camera path with a strong opening frame, depth-preserving movement, and no chaotic drifting.",
            motion_language: "Prefer confident reveal beats, subtle parallax, and edit-friendly pacing that can survive clipping, captions, and voiceover.",
            scene_construction: "Reconstruct the subject as layered, editable 3D forms instead of a flat imitation. Preserve hierarchy, silhouette, and major interaction surfaces.",
            delivery_goal: "Aim for a buyer-ready marketing asset that can be polished further with FFmpeg, subtitles, voiceover, and platform exports.",
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
            camera_language: "Keep framing intentional and easy to follow, with smooth transitions and clean spatial continuity.",
            motion_language: "Stage the animation as clear beats with an instantly readable opening, a strong middle reveal, and a clean finish frame.",
            scene_construction: "Build the animation from reusable visual elements, maintaining structure that remains editable for later revisions and compositing.",
            delivery_goal: "Aim for a polished sequence that can slot into explainers, launch videos, creator workflows, and premium client deliverables.",
        }
    }

    pub fn guidance_block(
        &self,
        production_context: Option<&str>,
        revision_notes: Option<&str>,
    ) -> String {
        let kind_line = match self.kind {
            BlenderOutputKind::Scene => {
                "Target a polished commercial 3D scene with readable staging, premium lighting, and purposeful camera motion."
            }
            BlenderOutputKind::Animation => {
                "Target a polished motion sequence with clean pacing, strong readability, and edit-friendly timing."
            }
        };

        let reference_line = if self.has_reference_image {
            "Treat the reference image as a layout-and-brand anchor. Preserve brand cues, layout hierarchy, and recognizable UI/product identity while rebuilding it as an editable scene."
        } else {
            "Create strong visual hierarchy even without a supplied brand reference."
        };

        let context_line = production_context
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("Extra production context: {value}."))
            .unwrap_or_default();

        let revision_line = revision_notes
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| format!("Apply these revision notes from prior feedback: {value}."))
            .unwrap_or_default();

        format!(
            "{kind_line} Style direction: {}. {} {} {} {} {} {} {} Review against: {}.",
            self.style,
            reference_line,
            self.scene_construction,
            self.camera_language,
            self.motion_language,
            self.delivery_goal,
            context_line,
            revision_line,
            self.review_checklist.join(", ")
        )
    }
}

pub fn enrich_scene_prompt(prompt: &str, style: &str, has_reference_image: bool) -> String {
    enrich_scene_prompt_with_context(prompt, style, has_reference_image, None, None)
}

pub fn enrich_scene_prompt_with_context(
    prompt: &str,
    style: &str,
    has_reference_image: bool,
    production_context: Option<&str>,
    revision_notes: Option<&str>,
) -> String {
    let brief = BlenderQualityBrief::for_scene(style, has_reference_image);
    format!(
        "{prompt}\n\nCreative direction: {}",
        brief.guidance_block(production_context, revision_notes)
    )
}

pub fn enrich_animation_description(description: &str, quality: &str) -> String {
    enrich_animation_description_with_context(description, quality, None, None)
}

pub fn enrich_animation_description_with_context(
    description: &str,
    quality: &str,
    production_context: Option<&str>,
    revision_notes: Option<&str>,
) -> String {
    let brief = BlenderQualityBrief::for_animation(quality);
    format!(
        "{description}\n\nCreative direction: {}",
        brief.guidance_block(production_context, revision_notes)
    )
}
