// This file is part of Astarte.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Errors returned by the MQTT connection

use rumqttc::ClientError;

use crate::{store::error::StoreError, topic::TopicError};

use super::{PairingError, PayloadError};

/// Errors raised during construction of the [`Mqtt`](super::Mqtt) struct
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum MqttError {
    /// Error while pairing with Astarte
    #[error("couldn't pair with Astarte")]
    Pairing(#[from] PairingError),
    #[error("Error while loading session data to perform the mqtt connection: {0}")]
    PropLoad(#[from] StoreError),
    /// Failed to subscribe to topic
    #[error["Couldn't subscribe to topic"]]
    Subscribe(#[source] ClientError),
    /// Failed to unsubscribe to topic
    #[error["Couldn't unsubscribe to topic"]]
    Unsubscribe(#[source] ClientError),
    /// Failed to publish on topic
    #[error("Couldn't publish on topic {ctx}")]
    Publish {
        ctx: &'static str,
        #[source]
        backtrace: ClientError,
    },
    /// Errors that can occur handling the payload.
    #[error("couldn't process payload")]
    Payload(#[from] PayloadError),
    /// Couldn't parse the topic
    #[error("couldn't parse the topic")]
    Topic(#[from] TopicError),
}

impl MqttError {
    pub(crate) const fn publish(ctx: &'static str, error: ClientError) -> Self {
        Self::Publish {
            ctx,
            backtrace: error,
        }
    }
}
