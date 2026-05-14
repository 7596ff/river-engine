//! HTTP-based adapter client

use crate::{
    Adapter, AdapterError, AdapterInfo, Feature, IncomingEvent, SendRequest, SendResponse,
};
use async_trait::async_trait;

/// Gateway-side client for external adapters
pub struct HttpAdapter {
    pub info: AdapterInfo,
    client: reqwest::Client,
}

impl HttpAdapter {
    pub fn new(info: AdapterInfo) -> Self {
        Self {
            info,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Adapter for HttpAdapter {
    fn name(&self) -> &str {
        &self.info.name
    }

    fn supports(&self, feature: &Feature) -> bool {
        self.info.features.contains(feature)
    }

    async fn send(&self, request: SendRequest) -> Result<SendResponse, AdapterError> {
        let url = format!("{}/send", self.info.url);
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .json()
            .await?;
        Ok(response)
    }

    async fn read_history(
        &self,
        channel: &str,
        limit: usize,
    ) -> Result<Vec<IncomingEvent>, AdapterError> {
        if !self.supports(&Feature::ReadHistory) {
            return Err(AdapterError::FeatureNotSupported(Feature::ReadHistory));
        }
        let url = format!("{}/history/{}?limit={}", self.info.url, channel, limit);
        let response = self.client.get(&url).send().await?.json().await?;
        Ok(response)
    }

    async fn health(&self) -> Result<bool, AdapterError> {
        let url = format!("{}/health", self.info.url);
        let response: serde_json::Value = self.client.get(&url).send().await?.json().await?;
        Ok(response
            .get("healthy")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }
}
