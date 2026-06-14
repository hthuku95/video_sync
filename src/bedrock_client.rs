use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConverseOutput, InferenceConfiguration, Message, SystemContentBlock,
    Tool, ToolConfiguration, ToolInputSchema, ToolResultContentBlock, ToolSpecification,
};
use aws_smithy_types::Document;
use aws_smithy_types::Number as DocNumber;
use serde_json::Value;
use std::collections::HashMap;

const DEFAULT_MODEL: &str = "meta.llama4-maverick-17b-instruct-v1:0";

#[derive(Debug, Clone)]
pub struct BedrockToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug)]
pub enum BedrockResponse {
    Text(String),
    ToolCalls(Vec<BedrockToolCall>),
}

#[derive(Clone)]
pub struct BedrockClient {
    client: aws_sdk_bedrockruntime::Client,
    model_id: String,
}

fn value_to_doc(v: &Value) -> Document {
    match v {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Document::Number(DocNumber::PosInt(i as u64))
                } else {
                    Document::Number(DocNumber::NegInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                Document::Number(DocNumber::Float(f))
            } else {
                Document::Null
            }
        }
        Value::String(s) => Document::String(s.clone()),
        Value::Array(arr) => Document::Array(arr.iter().map(value_to_doc).collect()),
        Value::Object(obj) => {
            let map: HashMap<String, Document> =
                obj.iter().map(|(k, v)| (k.clone(), value_to_doc(v))).collect();
            Document::Object(map)
        }
    }
}

fn doc_to_value(doc: &Document) -> Value {
    match doc {
        Document::Null => Value::Null,
        Document::Bool(b) => Value::Bool(*b),
        Document::Number(n) => {
            let f = n.to_f64_lossy();
            serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        Document::String(s) => Value::String(s.clone()),
        Document::Array(arr) => Value::Array(arr.iter().map(doc_to_value).collect()),
        Document::Object(obj) => {
            let map: serde_json::Map<String, Value> =
                obj.iter().map(|(k, v)| (k.clone(), doc_to_value(v))).collect();
            Value::Object(map)
        }
    }
}

impl BedrockClient {
    pub async fn new_async(region: &str, model_id: Option<String>) -> Self {
        let region = region.to_string();
        let config = aws_config::from_env()
            .region(aws_config::Region::new(region))
            .load()
            .await;
        let client = aws_sdk_bedrockruntime::Client::new(&config);
        let model_id = model_id.unwrap_or_else(|| DEFAULT_MODEL.to_string());
        tracing::info!("Bedrock client initialized: model={}", model_id);
        Self { client, model_id }
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub async fn generate_single(
        &self,
        system_prompt: &str,
        messages: &[Message],
        tools: &[crate::gemini_client::FunctionDeclaration],
    ) -> Result<BedrockResponse, String> {
        let mut builder = self.client.converse()
            .model_id(&self.model_id)
            .inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(4096)
                    .temperature(0.5)
                    .build(),
            );

        if !system_prompt.is_empty() {
            builder = builder
                .system(SystemContentBlock::Text(system_prompt.to_string()));
        }

        builder = builder.set_messages(Some(messages.to_vec()));

        if !tools.is_empty() {
            let bedrock_tools: Vec<Tool> = tools.iter().map(tool_decl_to_bedrock).collect();
            let tc = ToolConfiguration::builder()
                .set_tools(Some(bedrock_tools))
                .build()
                .unwrap();
            builder = builder.tool_config(tc);
        }

        let output = builder.send().await.map_err(|e| format!("Bedrock API error: {e}"))?;

        let converse_output = output
            .output
            .ok_or_else(|| "Bedrock: no output in response".to_string())?;

        let msg = match converse_output {
            ConverseOutput::Message(m) => m,
            _ => return Err("Bedrock: unexpected output variant".to_string()),
        };

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for block in &msg.content {
            match block {
                ContentBlock::Text(t) => text_parts.push(t.as_str()),
                ContentBlock::ToolUse(tu) => {
                    let arguments = doc_to_value(&tu.input);
                    tool_calls.push(BedrockToolCall {
                        id: tu.tool_use_id.clone(),
                        name: tu.name.clone(),
                        arguments,
                    });
                }
                _ => {}
            }
        }

        if !tool_calls.is_empty() {
            return Ok(BedrockResponse::ToolCalls(tool_calls));
        }

        Ok(BedrockResponse::Text(text_parts.join("\n")))
    }
}

fn tool_decl_to_bedrock(d: &crate::gemini_client::FunctionDeclaration) -> Tool {
    let props: serde_json::Map<String, Value> = d
        .parameters
        .properties
        .iter()
        .map(|(k, v)| {
            let mut prop = serde_json::json!({
                "type": v.prop_type,
                "description": v.description,
            });
            if let Some(ref items) = v.items {
                prop["items"] = serde_json::json!({ "type": items });
            }
            (k.clone(), prop)
        })
        .collect();

    let schema_value = serde_json::json!({
        "type": "object",
        "properties": props,
        "required": d.parameters.required,
    });

    let schema_doc = value_to_doc(&schema_value);

    Tool::ToolSpec(
        ToolSpecification::builder()
            .name(&d.name)
            .description(&d.description)
            .input_schema(ToolInputSchema::Json(schema_doc))
            .build()
            .unwrap(),
    )
}

pub fn tool_call_to_content_block(tc: &BedrockToolCall) -> ContentBlock {
    let input_doc = value_to_doc(&tc.arguments);
    ContentBlock::ToolUse(
        aws_sdk_bedrockruntime::types::ToolUseBlock::builder()
            .tool_use_id(&tc.id)
            .name(&tc.name)
            .input(input_doc)
            .build()
            .unwrap(),
    )
}

pub fn tool_result_to_content_block(tc: &BedrockToolCall, result_str: &str) -> ContentBlock {
    ContentBlock::ToolResult(
        aws_sdk_bedrockruntime::types::ToolResultBlock::builder()
            .tool_use_id(&tc.id)
            .content(ToolResultContentBlock::Text(result_str.to_string()))
            .build()
            .unwrap(),
    )
}
