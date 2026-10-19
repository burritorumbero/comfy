//! The exported workflow: which input takes the prompt, which take the seed (and, to edit a
//! picture, the image), which LoRAs can be switched off, and the per-run copy with all of
//! that filled in.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A ComfyUI API-format workflow: node id -> {"class_type", "inputs", "_meta"}.
pub type Graph = Map<String, Value>;
/// A node input: (node id, input name).
pub type Target = (String, String);

/// If the workflow's prompt text contains this, only this part is replaced.
pub const PLACEHOLDER: &str = "{prompt}";
/// A LoRA node titled "Option: Realism" becomes a switch named Realism in the app.
pub const OPTION_PREFIX: &str = "Option:";
const STRENGTH_INPUTS: &[&str] = &["strength_model", "strength_clip", "strength"];

/// What a workflow does: make a picture from the prompt, or change the one it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Generate,
    Edit,
}

impl Kind {
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Generate => "image generation",
            Kind::Edit => "image editing",
        }
    }
}

/// Inputs that hold prompt text, most specific first.
const TEXT_INPUTS: &[&str] = &[
    "text",
    "prompt",
    "text_g",
    "text_l",
    "positive",
    "positive_prompt",
    "string",
    "value",
];
/// Inputs that carry the positive conditioning between nodes.
const CONDITIONING_INPUTS: &[&str] = &["conditioning", "conditioning_to", "conditioning_1", "guider", "positive"];
const SEED_INPUTS: &[&str] = &["seed", "noise_seed"];
/// Where a seed input that is wired to another node finds its number.
const SEED_SOURCE_INPUTS: &[&str] = &["seed", "noise_seed", "value", "int"];

#[derive(Debug, PartialEq)]
pub struct WorkflowError(pub String);

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn error<T>(message: impl Into<String>) -> Result<T, WorkflowError> {
    Err(WorkflowError(message.into()))
}

/// API-format inputs wired to another node look like ["node id", output slot].
fn link(value: Option<&Value>) -> Option<&str> {
    match value?.as_array()?.as_slice() {
        [Value::String(id), Value::Number(slot)] if slot.is_u64() || slot.is_i64() => Some(id),
        _ => None,
    }
}

fn is_int(value: Option<&Value>) -> bool {
    value.is_some_and(|v| v.is_u64() || v.is_i64())
}

fn inputs<'a>(graph: &'a Graph, node_id: &str) -> Option<&'a Map<String, Value>> {
    graph.get(node_id)?.get("inputs")?.as_object()
}

fn class_type<'a>(graph: &'a Graph, node_id: &str) -> &'a str {
    graph
        .get(node_id)
        .and_then(|n| n.get("class_type"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn title<'a>(graph: &'a Graph, node_id: &str) -> Option<&'a str> {
    graph.get(node_id)?.get("_meta")?.get("title")?.as_str()
}

/// `238:227 "CLIP Text Encode (Positive Prompt)" (CLIPTextEncode)`, for messages.
pub fn node_label(graph: &Graph, node_id: &str) -> String {
    let class = class_type(graph, node_id);
    match title(graph, node_id) {
        Some(t) if t != class => format!("{node_id} \"{t}\" ({class})"),
        _ => format!("{node_id} ({class})"),
    }
}

pub fn load(path: &Path) -> Result<Graph, WorkflowError> {
    let shown = path.display();
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return error(format!("{shown} does not exist.")),
        Err(e) => return error(format!("{shown} is not readable: {e}")),
    };
    let data: Value = match serde_json::from_str(&raw) {
        Ok(data) => data,
        Err(e) => return error(format!("{shown} is not readable JSON: {e}")),
    };
    let Value::Object(mut graph) = data else {
        return error(format!("{shown} is not an API-format ComfyUI workflow."));
    };
    if graph.contains_key("nodes") && graph.contains_key("links") {
        return error(format!(
            "{shown} is the editor format. Open the workflow in ComfyUI and use File > Export (API) instead, \
             then point --workflow at that file."
        ));
    }
    // Someone saved a whole /prompt request body.
    if let Some(Value::Object(prompt)) = graph.remove("prompt") {
        graph = prompt;
    }
    let valid = !graph.is_empty() && graph.values().all(|n| n.get("class_type").is_some_and(Value::is_string));
    if !valid {
        return error(format!("{shown} is not an API-format ComfyUI workflow."));
    }
    for node in graph.values_mut() {
        let node = node.as_object_mut().expect("checked above");
        if !node.get("inputs").is_some_and(Value::is_object) {
            node.insert("inputs".into(), Value::Object(Map::new()));
        }
    }
    Ok(graph)
}

/// Walk back from a node to the input that holds its prompt text.
fn find_text(graph: &Graph, node_id: &str, seen: &mut HashSet<String>, input_name: Option<&str>) -> Option<Target> {
    if seen.len() > 64 || !seen.insert(node_id.to_string()) {
        return None;
    }
    let inputs = inputs(graph, node_id)?;
    let names: Vec<&str> = match input_name {
        Some(name) => vec![name],
        None => TEXT_INPUTS.to_vec(),
    };
    for name in &names {
        if inputs.get(*name).is_some_and(Value::is_string) {
            return Some((node_id.to_string(), name.to_string()));
        }
    }
    let followed = names.iter().copied().chain(if input_name.is_none() {
        CONDITIONING_INPUTS.iter().copied()
    } else {
        [].iter().copied()
    });
    for name in followed {
        if let Some(source) = link(inputs.get(name)) {
            if let Some(found) = find_text(graph, source, seen, None) {
                return Some(found);
            }
        }
    }
    None
}

/// The inputs the user's prompt is written into.
///
/// Without an explicit node, follow every sampler's (or guider's) positive conditioning
/// back to the text that feeds it. Negative prompts are never reached because the walk
/// only starts from positive inputs.
pub fn find_prompt_targets(graph: &Graph, node_id: Option<&str>, input_name: Option<&str>) -> Result<Vec<Target>, WorkflowError> {
    if let Some(node_id) = node_id {
        if !graph.contains_key(node_id) {
            let ids: Vec<&str> = graph.keys().map(String::as_str).collect();
            return error(format!(
                "--prompt-node {node_id} is not in the workflow. Node ids: {}",
                ids.join(", ")
            ));
        }
        return match find_text(graph, node_id, &mut HashSet::new(), input_name) {
            Some(found) => Ok(vec![found]),
            None => {
                let what = input_name.map_or("text input".to_string(), |n| format!("input '{n}'"));
                error(format!("Node {} has no {what} to put the prompt in.", node_label(graph, node_id)))
            }
        };
    }

    let mut starts = Vec::new();
    for (id, node) in graph {
        let Some(inputs) = node.get("inputs").and_then(Value::as_object) else {
            continue;
        };
        if let Some(source) = link(inputs.get("positive")) {
            starts.push(source);
        } else if class_type(graph, id).ends_with("Guider") {
            if let Some(source) = link(inputs.get("conditioning")) {
                starts.push(source);
            }
        }
    }
    let mut targets = Vec::new();
    for start in starts {
        if let Some(found) = find_text(graph, start, &mut HashSet::new(), None) {
            if !targets.contains(&found) {
                targets.push(found);
            }
        }
    }
    if !targets.is_empty() {
        return Ok(targets);
    }

    // No sampler to start from: fall back to a node titled as the positive prompt.
    for id in graph.keys() {
        if title(graph, id).is_some_and(|t| t.to_lowercase().contains("positive")) {
            if let Some(found) = find_text(graph, id, &mut HashSet::new(), None) {
                return Ok(vec![found]);
            }
        }
    }
    let encoders: Vec<&String> = graph.keys().filter(|id| class_type(graph, id) == "CLIPTextEncode").collect();
    if let [only] = encoders.as_slice() {
        if let Some(found) = find_text(graph, only, &mut HashSet::new(), None) {
            return Ok(vec![found]);
        }
    }
    error(
        "Could not tell which node holds the positive prompt. Pass --prompt-node with its id \
         (the number on the node in ComfyUI, or the key in the exported JSON).",
    )
}

/// The inputs that hold a fixed seed number.
pub fn find_seed_targets(graph: &Graph) -> Vec<Target> {
    let mut targets = Vec::new();
    for (id, node) in graph {
        let Some(inputs) = node.get("inputs").and_then(Value::as_object) else {
            continue;
        };
        for name in SEED_INPUTS {
            let value = inputs.get(*name);
            let target = if is_int(value) {
                Some((id.clone(), name.to_string()))
            } else if let Some(source) = link(value) {
                inputs_of_source(graph, source)
            } else {
                None
            };
            if let Some(target) = target {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
    }
    targets
}

fn inputs_of_source(graph: &Graph, source: &str) -> Option<Target> {
    let inputs = inputs(graph, source)?;
    SEED_SOURCE_INPUTS
        .iter()
        .find(|name| is_int(inputs.get(**name)))
        .map(|name| (source.to_string(), name.to_string()))
}

/// The inputs that take the picture to edit: each Load Image node's `image`.
pub fn find_image_targets(graph: &Graph) -> Vec<Target> {
    graph
        .keys()
        .filter(|id| class_type(graph, id) == "LoadImage")
        .filter(|id| inputs(graph, id).is_some_and(|i| i.get("image").is_some_and(Value::is_string)))
        .map(|id| (id.clone(), "image".to_string()))
        .collect()
}

/// A LoRA the app can switch off, from a node titled "Option: <name>".
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkflowOption {
    pub name: String,
    /// Whether it is on when a request doesn't say: its strength in the workflow isn't 0.
    pub on: bool,
    /// The nodes it switches, with their strength inputs and the strengths saved in the workflow.
    #[serde(skip)]
    strengths: Vec<(Target, f64)>,
}

pub fn find_options(graph: &Graph) -> Vec<WorkflowOption> {
    let mut options: Vec<WorkflowOption> = Vec::new();
    for id in graph.keys() {
        let Some(name) = title(graph, id).and_then(|t| t.strip_prefix(OPTION_PREFIX)).map(str::trim) else {
            continue;
        };
        let Some(inputs) = inputs(graph, id) else { continue };
        let strengths: Vec<(Target, f64)> = STRENGTH_INPUTS
            .iter()
            .filter_map(|input| Some(((id.clone(), input.to_string()), inputs.get(*input)?.as_f64()?)))
            .collect();
        if name.is_empty() || strengths.is_empty() {
            continue;
        }
        let on = strengths.iter().any(|(_, s)| *s != 0.0);
        // Nodes that share a name switch together.
        match options.iter_mut().find(|o| o.name == name) {
            Some(option) => {
                option.on |= on;
                option.strengths.extend(strengths);
            }
            None => options.push(WorkflowOption {
                name: name.to_string(),
                on,
                strengths,
            }),
        }
    }
    options
}

/// One run's inputs besides the workflow.
pub struct Run<'a> {
    pub prompt: &'a str,
    pub seed: u64,
    /// The picture to edit, as ComfyUI's Load Image names it ("prompt-app/x.png").
    pub image: Option<&'a str>,
    /// Options switched on or off by name; the rest keep their default.
    pub options: &'a HashMap<String, bool>,
}

pub struct Workflow {
    pub path: PathBuf,
    pub kind: Kind,
    pub graph: Graph,
    pub prompt_targets: Vec<Target>,
    pub seed_targets: Vec<Target>,
    pub image_targets: Vec<Target>,
    pub options: Vec<WorkflowOption>,
    pub title: String,
}

impl Workflow {
    pub fn open(
        path: &Path,
        kind: Kind,
        prompt_node: Option<&str>,
        prompt_input: Option<&str>,
        keep_seed: bool,
        title: Option<String>,
    ) -> Result<Self, WorkflowError> {
        let graph = load(path)?;
        let prompt_targets = find_prompt_targets(&graph, prompt_node, prompt_input)?;
        let seed_targets = if keep_seed { Vec::new() } else { find_seed_targets(&graph) };
        let image_targets = find_image_targets(&graph);
        if kind == Kind::Edit && image_targets.is_empty() {
            return error(format!("{} has no Load Image node to take the picture to edit.", path.display()));
        }
        let options = find_options(&graph);
        let title = title.unwrap_or_else(|| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("ComfyUI");
            match stem.to_lowercase().as_str() {
                "workflow_api" | "workflow" | "prompt" => "ComfyUI".to_string(),
                _ => stem.to_string(),
            }
        });
        Ok(Workflow {
            path: path.to_path_buf(),
            kind,
            graph,
            prompt_targets,
            seed_targets,
            image_targets,
            options,
            title,
        })
    }

    /// The options a run uses: each one's default unless `chosen` names it.
    pub fn options_for(&self, chosen: &HashMap<String, bool>) -> Vec<(&str, bool)> {
        self.options
            .iter()
            .map(|o| (o.name.as_str(), chosen.get(&o.name).copied().unwrap_or(o.on)))
            .collect()
    }

    /// A copy of the workflow with the prompt, the picture and the options written in and,
    /// unless the seed is kept, the run's seed in every seed input. Returns the seed used.
    pub fn build(&self, run: &Run) -> (Graph, Option<u64>) {
        let Run { prompt, seed, .. } = *run;
        let mut graph = self.graph.clone();
        if let Some(image) = run.image {
            for (id, name) in &self.image_targets {
                graph[id]["inputs"][name] = Value::String(image.to_string());
            }
        }
        for option in &self.options {
            let on = run.options.get(&option.name).copied().unwrap_or(option.on);
            for ((id, name), saved) in &option.strengths {
                // Switched on with no strength saved: full strength.
                let strength = match (on, *saved) {
                    (false, _) => 0.0,
                    (true, 0.0) => 1.0,
                    (true, saved) => saved,
                };
                graph[id]["inputs"][name] = Value::from(strength);
            }
        }
        for (id, name) in &self.prompt_targets {
            let slot = &mut graph[id]["inputs"][name];
            let template = slot.as_str().unwrap_or_default();
            *slot = Value::String(if template.contains(PLACEHOLDER) {
                template.replace(PLACEHOLDER, prompt)
            } else {
                prompt.to_string()
            });
        }
        if self.seed_targets.is_empty() {
            return (graph, None);
        }
        for (id, name) in &self.seed_targets {
            graph[id]["inputs"][name] = Value::from(seed);
        }
        (graph, Some(seed))
    }
}

/// `--workflow`, else `workflow_api.json` in `dir`, else the only `.json` file there.
pub fn resolve_path(explicit: Option<PathBuf>, dir: &Path) -> Result<PathBuf, WorkflowError> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let default = dir.join("workflow_api.json");
    if default.exists() {
        return Ok(default);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => error(format!(
            "No workflow found. In ComfyUI use File > Export (API), save the file into {}, or pass --workflow with its path.",
            dir.display()
        )),
        _ => {
            let names: Vec<String> = found
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
            error(format!(
                "Several workflows in {} ({}). Pass --workflow to pick one.",
                dir.display(),
                names.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qwen_image_2512_api.json")
    }

    fn graph(value: Value) -> Graph {
        value.as_object().unwrap().clone()
    }

    fn t(id: &str, name: &str) -> Target {
        (id.to_string(), name.to_string())
    }

    static NO_OPTIONS: std::sync::LazyLock<HashMap<String, bool>> = std::sync::LazyLock::new(HashMap::new);
    fn run(prompt: &str, seed: u64) -> Run<'_> {
        Run {
            prompt,
            seed,
            image: None,
            options: &NO_OPTIONS,
        }
    }

    fn edit_fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("workflows/qwen_image_edit_2511_api.json")
    }

    fn write(dir: &Path, name: &str, value: Value) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, value.to_string()).unwrap();
        path
    }

    #[test]
    fn qwen_image_2512_prompt_and_seed() {
        // The official template, exported by ComfyUI's own frontend with its subgraph flattened.
        let graph = load(&fixture()).unwrap();
        assert_eq!(find_prompt_targets(&graph, None, None).unwrap(), vec![t("238:227", "text")]);
        assert_eq!(find_seed_targets(&graph), vec![t("238:230", "seed")]);
    }

    #[test]
    fn qwen_image_edit_2511_prompt_seed_picture_and_realism() {
        let workflow = Workflow::open(&edit_fixture(), Kind::Edit, None, None, false, None).unwrap();
        assert_eq!(
            workflow.prompt_targets,
            vec![t("10", "prompt")],
            "the negative encoder is left alone"
        );
        assert_eq!(workflow.seed_targets, vec![t("15", "seed")]);
        assert_eq!(workflow.image_targets, vec![t("8", "image")]);
        assert_eq!(
            workflow.options.iter().map(|o| (o.name.as_str(), o.on)).collect::<Vec<_>>(),
            [("Realism", true)]
        );

        let off = HashMap::from([("Realism".to_string(), false)]);
        let (built, _) = workflow.build(&Run {
            prompt: "make it night, with stars",
            seed: 9,
            image: Some("prompt-app/abc.png"),
            options: &off,
        });
        assert_eq!(built["10"]["inputs"]["prompt"], "make it night, with stars");
        assert_eq!(built["11"]["inputs"]["prompt"], "");
        assert_eq!(built["8"]["inputs"]["image"], "prompt-app/abc.png");
        assert_eq!(built["7"]["inputs"]["strength_model"], 0.0);
        assert_eq!(built["6"]["inputs"]["strength_model"], 1, "Lightning isn't an option");
        let (on, _) = workflow.build(&run("x", 1));
        assert_eq!(on["7"]["inputs"]["strength_model"], 0.85);
        assert_eq!(workflow.options_for(&off), [("Realism", false)]);
    }

    #[test]
    fn options_switched_on_from_zero_get_full_strength_and_share_names() {
        let g = graph(json!({
            "1": {"class_type": "LoraLoader", "_meta": {"title": "Option: Grain"}, "inputs": {"strength_model": 0, "strength_clip": 0}},
            "2": {"class_type": "LoraLoaderModelOnly", "_meta": {"title": "Option:Grain"}, "inputs": {"strength_model": 0.5}},
            "3": {"class_type": "LoraLoaderModelOnly", "_meta": {"title": "Option: "}, "inputs": {"strength_model": 1}},
        }));
        let options = find_options(&g);
        assert_eq!(options.len(), 1, "a nameless option is ignored");
        assert_eq!((options[0].name.as_str(), options[0].on), ("Grain", true));
        let workflow = Workflow {
            path: "w.json".into(),
            kind: Kind::Generate,
            prompt_targets: vec![],
            seed_targets: vec![],
            image_targets: vec![],
            options,
            graph: g,
            title: "t".into(),
        };
        let (built, _) = workflow.build(&run("x", 1));
        assert_eq!(built["1"]["inputs"]["strength_clip"], 1.0);
        assert_eq!(built["2"]["inputs"]["strength_model"], 0.5);
    }

    #[test]
    fn an_edit_workflow_needs_a_load_image_node() {
        assert!(
            Workflow::open(&fixture(), Kind::Edit, None, None, false, None)
                .err()
                .unwrap()
                .0
                .contains("Load Image")
        );
    }

    #[test]
    fn build_changes_only_the_prompt_and_seed() {
        let workflow = Workflow::open(&fixture(), Kind::Generate, None, None, false, None).unwrap();
        let (built, seed) = workflow.build(&run("a red fox in the snow", 1234));
        assert_eq!(seed, Some(1234));
        assert_eq!(built["238:227"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(built["238:230"]["inputs"]["seed"], 1234);
        let mut restored = built.clone();
        restored["238:227"]["inputs"]["text"] = workflow.graph["238:227"]["inputs"]["text"].clone();
        restored["238:230"]["inputs"]["seed"] = workflow.graph["238:230"]["inputs"]["seed"].clone();
        assert_eq!(restored, workflow.graph, "nothing else changes");
    }

    #[test]
    fn keep_seed_leaves_it_alone() {
        let workflow = Workflow::open(&fixture(), Kind::Generate, None, None, true, None).unwrap();
        let (built, seed) = workflow.build(&run("x", 1));
        assert_eq!(seed, None);
        assert_eq!(built["238:230"]["inputs"]["seed"], workflow.graph["238:230"]["inputs"]["seed"]);
    }

    #[test]
    fn placeholder_keeps_the_rest_of_the_text() {
        let g = graph(json!({
            "3": {"class_type": "KSampler", "inputs": {"seed": 1, "positive": ["6", 0], "negative": ["7", 0]}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "RAW photo, {prompt}, 85mm"}},
            "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "cartoon"}},
        }));
        let workflow = Workflow {
            path: "w.json".into(),
            kind: Kind::Generate,
            prompt_targets: find_prompt_targets(&g, None, None).unwrap(),
            seed_targets: vec![],
            image_targets: vec![],
            options: vec![],
            graph: g,
            title: "t".into(),
        };
        let (built, _) = workflow.build(&run("a lighthouse", 0));
        assert_eq!(built["6"]["inputs"]["text"], "RAW photo, a lighthouse, 85mm");
        assert_eq!(built["7"]["inputs"]["text"], "cartoon");
    }

    #[test]
    fn flux_guider_chain() {
        let g = graph(json!({
            "13": {"class_type": "SamplerCustomAdvanced", "inputs": {"noise": ["25", 0], "guider": ["22", 0]}},
            "22": {"class_type": "BasicGuider", "inputs": {"model": ["12", 0], "conditioning": ["26", 0]}},
            "26": {"class_type": "FluxGuidance", "inputs": {"guidance": 3.5, "conditioning": ["6", 0]}},
            "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "", "clip": ["11", 0]}},
            "25": {"class_type": "RandomNoise", "inputs": {"noise_seed": 7}},
        }));
        assert_eq!(find_prompt_targets(&g, None, None).unwrap(), vec![t("6", "text")]);
        assert_eq!(find_seed_targets(&g), vec![t("25", "noise_seed")]);
    }

    #[test]
    fn cfg_guider_with_text_from_a_primitive_node() {
        let g = graph(json!({
            "1": {"class_type": "SamplerCustomAdvanced", "inputs": {"guider": ["2", 0]}},
            "2": {"class_type": "CFGGuider", "inputs": {"cfg": 4, "positive": ["3", 0], "negative": ["4", 0]}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"text": ["5", 0]}},
            "4": {"class_type": "CLIPTextEncode", "inputs": {"text": "bad"}},
            "5": {"class_type": "PrimitiveStringMultiline", "inputs": {"value": "hello"}},
        }));
        assert_eq!(find_prompt_targets(&g, None, None).unwrap(), vec![t("5", "value")]);
    }

    #[test]
    fn two_samplers_sharing_one_prompt_write_it_once() {
        let g = graph(json!({
            "1": {"class_type": "KSampler", "inputs": {"seed": 1, "positive": ["3", 0], "negative": ["4", 0]}},
            "2": {"class_type": "KSampler", "inputs": {"seed": ["9", 0], "positive": ["3", 0], "negative": ["4", 0]}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"text": "a"}},
            "4": {"class_type": "CLIPTextEncode", "inputs": {"text": "b"}},
            "9": {"class_type": "PrimitiveInt", "inputs": {"value": 5}},
        }));
        assert_eq!(find_prompt_targets(&g, None, None).unwrap(), vec![t("3", "text")]);
        assert_eq!(find_seed_targets(&g), vec![t("1", "seed"), t("9", "value")]);
    }

    #[test]
    fn titled_node_without_a_sampler() {
        let g = graph(json!({
            "1": {"class_type": "CLIPTextEncode", "_meta": {"title": "Negative"}, "inputs": {"text": "b"}},
            "2": {"class_type": "CLIPTextEncode", "_meta": {"title": "Positive Prompt"}, "inputs": {"text": "a"}},
        }));
        assert_eq!(find_prompt_targets(&g, None, None).unwrap(), vec![t("2", "text")]);
    }

    #[test]
    fn ambiguous_workflow_asks_for_prompt_node() {
        let g = graph(json!({
            "1": {"class_type": "CLIPTextEncode", "inputs": {"text": "a"}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"text": "b"}},
        }));
        assert!(find_prompt_targets(&g, None, None).unwrap_err().0.contains("--prompt-node"));
    }

    #[test]
    fn explicit_node() {
        let g = graph(json!({
            "1": {"class_type": "CLIPTextEncode", "inputs": {"text": "a"}},
            "2": {"class_type": "CustomPrompt", "inputs": {"prompt": "b", "style": "x"}},
        }));
        assert_eq!(find_prompt_targets(&g, Some("2"), None).unwrap(), vec![t("2", "prompt")]);
        assert_eq!(find_prompt_targets(&g, Some("2"), Some("style")).unwrap(), vec![t("2", "style")]);
        assert!(
            find_prompt_targets(&g, Some("99"), None)
                .unwrap_err()
                .0
                .contains("not in the workflow")
        );
    }

    #[test]
    fn editor_format_is_refused_with_a_pointer_to_export_api() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "w.json", json!({"nodes": [], "links": [], "version": 0.4}));
        assert!(load(&path).unwrap_err().0.contains("Export (API)"));
    }

    #[test]
    fn a_saved_prompt_request_body_is_unwrapped() {
        let dir = tempfile::tempdir().unwrap();
        let inner = json!({"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "x"}}});
        let path = write(dir.path(), "w.json", json!({"prompt": inner, "client_id": "x"}));
        assert_eq!(Value::Object(load(&path).unwrap()), inner);
        let other = write(dir.path(), "o.json", json!([1, 2, 3]));
        assert!(load(&other).is_err());
    }

    #[test]
    fn the_only_json_file_is_used_and_names_the_page() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path(None, dir.path()).unwrap_err().0.contains("Export (API)"));
        let path = write(
            dir.path(),
            "Qwen-2512 BF16 Realistic.json",
            json!({"1": {"class_type": "CLIPTextEncode", "inputs": {"text": ""}}}),
        );
        assert_eq!(resolve_path(None, dir.path()).unwrap(), path);
        assert_eq!(
            Workflow::open(&path, Kind::Generate, None, None, false, None).unwrap().title,
            "Qwen-2512 BF16 Realistic"
        );
        write(dir.path(), "other.json", json!({}));
        assert!(resolve_path(None, dir.path()).unwrap_err().0.contains("Several workflows"));
    }
}
