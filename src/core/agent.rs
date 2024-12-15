use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use rig::agent::Agent as RigAgent;
use rig::completion::Prompt;
use rig::providers::anthropic::completion::CompletionModel;
use rig::providers::anthropic::{self, CLAUDE_3_HAIKU};
use serde_json::json;
use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
}; // Add this import at the top of your file
use teloxide::{
    prelude::*,
    types::{Message, Update},
};
pub struct Agent {
    agent: RigAgent<CompletionModel>,
    anthropic_api_key: String,
    prompt: String,
}

#[derive(Debug, PartialEq)]
pub enum ResponseDecision {
    Respond,
    Ignore,
}

impl Agent {
    pub fn new(anthropic_api_key: &str, prompt: &str) -> Self {
        let client = anthropic::ClientBuilder::new(anthropic_api_key).build();
        let agent = client
            .agent(CLAUDE_3_HAIKU)
            .preamble(prompt)
            .temperature(0.5)
            .max_tokens(4096)
            .build();
        Agent { 
            agent,
            anthropic_api_key: anthropic_api_key.to_string(),
            prompt: prompt.to_string(),
        }
    }

    pub async fn should_respond(&self, tweet: &str) -> Result<ResponseDecision, anyhow::Error> {
        let prompt = format!(
            "Tweet: {tweet}\n\
            Task: Reply [RESPOND] or [IGNORE] based on:\n\
            [RESPOND] if:\n\
            - Direct mention/address\n\
            - Contains question\n\
            - Contains command/request\n\
            [IGNORE] if:\n\
            - Unrelated content\n\
            - Spam/nonsensical\n\
            Answer:"
        );
        let response = self.agent.prompt(&prompt).await?;
        let response = response.to_uppercase();
        Ok(if response.contains("[RESPOND]") {
            ResponseDecision::Respond
        } else {
            ResponseDecision::Ignore
        })
    }

    pub async fn generate_reply(&self, tweet: &str) -> Result<String, anyhow::Error> {
        let prompt = format!(
            "Task: Generate a post/reply in your voice, style and perspective while using this as context:\n\
            Current Post: '{}'\n\
            Generate a brief, single response that:\n\
            - Uses all lowercase\n\
            - Avoids punctuation\n\
            - Is direct and possibly sarcastic\n\
            - Stays under 280 characters\n\
            Write only the response text, nothing else:",
            tweet
        );
        let response = self.agent.prompt(&prompt).await?;
        Ok(response.trim().to_string())
    }

    pub async fn generate_post(&self) -> Result<String, anyhow::Error> {
        let prompt = r#"# Task: Write a Social Media Post
            Write a 1-3 sentence post that would be engaging to readers. Keep it casual and friendly in tone. Stay under 280 characters.

            Requirements:
            - Write only the post content, no additional commentary
            - No emojis
            - No hashtags
            - No questions
            - No introductory phrases or meta-commentary
            - Brief, concise statements only
            - Focus on personal experiences, observations, or thoughts"#;
        let response = self.agent.prompt(&prompt).await?;
        Ok(response.trim().to_string())
    }
    pub async fn generate_image(&self) -> Result<String, anyhow::Error> {
        let client = reqwest::Client::builder().build()?;
        dotenv::dotenv().ok();
        let heuris_api = env::var("HEURIS_API")
            .map_err(|_| anyhow::anyhow!("HEURIS_API not found in environment"))?;
        let base_prompt = env::var("IMAGE_PROMPT")
            .map_err(|_| anyhow::anyhow!("IMAGE_PROMPT not found in environment"))?;
        let deadline = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 300;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Authorization", format!("Bearer {}", heuris_api).parse()?);
        headers.insert("Content-Type", "application/json".parse()?);

        let body = json!({
            "model_input": {
                "SD": {
                    "width": 1024,
                    "height": 1024,
                    "prompt": format!("{}", base_prompt),
                    "neg_prompt": "worst quality, bad quality, umbrella, blurry face, anime, illustration",
                    "num_iterations": 22,
                    "guidance_scale": 7.5
                }
            },
            "model_id": "BluePencilRealistic",
            "deadline": deadline,
            "priority": 1,
            "job_id": format!("job_{}", SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())
        });

        
        let request = client
            .request(
                reqwest::Method::POST,
                "http://sequencer.heurist.xyz/submit_job",
            )
            .headers(headers)
            .json(&body);

        let response = request.send().await?;
        let body = response.text().await?;
        Ok(body.trim_matches('"').to_string())
    }

    pub async fn prepare_image_for_tweet(&self, image_url: &str) -> Result<Vec<u8>, anyhow::Error> {
        let client = reqwest::Client::new();
        let response = client.get(image_url).send().await?;

        Ok(response.bytes().await?.to_vec())
    }

    pub async fn handle_telegram_message(&self, bot: &Bot) {
        let client = anthropic::ClientBuilder::new(&self.anthropic_api_key).build();
        let bot = bot.clone();
        let agent_prompt = self.prompt.clone();
        teloxide::repl(bot, move |bot: Bot, msg: Message| {
            let agent = client
                .agent(CLAUDE_3_HAIKU)
                .preamble(&agent_prompt)
                .temperature(0.5)
                .max_tokens(4096)
                .build();
            async move {
                if let Some(text) = msg.text() {
                    let should_respond = msg.chat.is_private() || text.contains("@rina_rig_bot");
                    
                    if should_respond {
                        let combined_prompt = format!(
                            "Task: Generate a conversational reply to this Telegram message while using this as context:\n\
                            Message: '{}'\n\
                            Generate a natural response that:\n\
                            - Is friendly and conversational\n\
                            - Can use normal punctuation and capitalization\n\
                            - May include emojis when appropriate\n\
                            - Maintains a helpful and engaging tone\n\
                            - Keeps responses concise but not artificially limited\n\
                            Write only the response text, nothing else:",
                            text
                        );
                        let response = agent
                            .prompt(&combined_prompt)
                            .await
                            .expect("Error generating the response");
                        println!("Telegram response: {}", response);
                        bot.send_message(msg.chat.id, response).await?;
                    }
                }
                Ok(())
            }
        })
        .await;
    }
}

