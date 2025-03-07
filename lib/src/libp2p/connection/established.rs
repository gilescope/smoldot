// Smoldot
// Copyright (C) 2019-2022  Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

mod multi_stream;
mod single_stream;
pub mod substream;
mod tests;

use super::yamux;
use alloc::{string::String, vec::Vec};
use core::time::Duration;

pub use multi_stream::{MultiStream, SubstreamFate};
pub use single_stream::{ConnectionPrototype, Error, SingleStream};
pub use substream::{
    InboundError, NotificationsInClosedErr, NotificationsOutErr, RequestError,
    RespondInRequestError,
};

/// Identifier of a request or a notifications substream.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubstreamId(SubstreamIdInner);

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SubstreamIdInner {
    SingleStream(yamux::SubstreamId),
    MultiStream(u32),
}

impl SubstreamId {
    /// Returns the value that compares inferior or equal to all possible values.
    pub fn min_value() -> Self {
        debug_assert!(
            SubstreamIdInner::SingleStream(yamux::SubstreamId::max_value())
                < SubstreamIdInner::MultiStream(0)
        );

        Self(SubstreamIdInner::SingleStream(
            yamux::SubstreamId::min_value(),
        ))
    }

    /// Returns the value that compares superior or equal to all possible values.
    pub fn max_value() -> Self {
        debug_assert!(
            SubstreamIdInner::MultiStream(0)
                > SubstreamIdInner::SingleStream(yamux::SubstreamId::max_value())
        );

        Self(SubstreamIdInner::MultiStream(u32::max_value()))
    }
}

/// Event that happened on the connection. See [`SingleStream::read_write`] and
/// [`MultiStream::pull_event`].
#[must_use]
#[derive(Debug)]
pub enum Event<TRqUd, TNotifUd> {
    /// The connection is now in a mode where opening new substreams (i.e. starting requests
    /// and opening notifications substreams) is forbidden, but the remote is still able to open
    /// substreams and messages on existing substreams are still allowed to be sent and received.
    NewOutboundSubstreamsForbidden,

    /// Received an incoming substream, but this substream has produced an error.
    ///
    /// > **Note**: This event exists only for diagnostic purposes. No action is expected in
    /// >           return.
    InboundError(InboundError),

    /// Received a request in the context of a request-response protocol.
    RequestIn {
        /// Identifier of the request. Needs to be provided back when answering the request.
        id: SubstreamId,
        /// Index of the request-response protocol the request was sent on.
        ///
        /// The index refers to the position of the protocol in [`Config::request_protocols`].
        protocol_index: usize,
        /// Bytes of the request. Its interpretation is out of scope of this module.
        request: Vec<u8>,
    },

    /// Received a response to a previously emitted request on a request-response protocol.
    Response {
        /// Bytes of the response. Its interpretation is out of scope of this module.
        response: Result<Vec<u8>, RequestError>,
        /// Identifier of the request. Value that was returned by [`SingleStream::add_request`]
        /// or [`MultiStream::add_request`].
        id: SubstreamId,
        /// Value that was passed to [`SingleStream::add_request`] or [`MultiStream::add_request`].
        user_data: TRqUd,
    },

    /// Remote has opened an inbound notifications substream.
    ///
    /// Either [`SingleStream::accept_in_notifications_substream`] or
    /// [`SingleStream::reject_in_notifications_substream`], or
    /// [`MultiStream::accept_in_notifications_substream`] or
    /// [`MultiStream::reject_in_notifications_substream`] must be called in the near future in
    /// order to accept or reject this substream.
    NotificationsInOpen {
        /// Identifier of the substream. Needs to be provided back when accept or rejecting the
        /// substream.
        id: SubstreamId,
        /// Index of the notifications protocol concerned by the substream.
        ///
        /// The index refers to the position of the protocol in
        /// [`Config::notifications_protocols`].
        protocol_index: usize,
        /// Handshake sent by the remote. Its interpretation is out of scope of this module.
        handshake: Vec<u8>,
    },
    /// Remote has canceled an inbound notifications substream opening.
    ///
    /// This can only happen after [`Event::NotificationsInOpen`].
    /// [`SingleStream::accept_in_notifications_substream`] or
    /// [`SingleStream::reject_in_notifications_substream`], or
    /// [`MultiStream::accept_in_notifications_substream`] or
    /// [`MultiStream::reject_in_notifications_substream`] should not be called on this substream.
    NotificationsInOpenCancel {
        /// Identifier of the substream.
        id: SubstreamId,
    },
    /// Remote has sent a notification on an inbound notifications substream. Can only happen
    /// after the substream has been accepted.
    // TODO: give a way to back-pressure notifications
    NotificationIn {
        /// Identifier of the substream.
        id: SubstreamId,
        /// Notification sent by the remote.
        notification: Vec<u8>,
    },
    /// Remote has closed an inbound notifications substream.Can only happen
    /// after the substream has been accepted.
    NotificationsInClose {
        /// Identifier of the substream.
        id: SubstreamId,
        /// If `Ok`, the substream has been closed gracefully. If `Err`, a problem happened.
        outcome: Result<(), NotificationsInClosedErr>,
    },

    /// Outcome of trying to open a substream with [`SingleStream::open_notifications_substream`]
    /// or [`MultiStream::open_notifications_substream`].
    ///
    /// If `Ok`, it is now possible to send notifications on this substream.
    /// If `Err`, the substream no longer exists.
    NotificationsOutResult {
        /// Identifier of the substream. Value that was returned by
        /// [`SingleStream::open_notifications_substream`] or
        /// [`MultiStream::open_notifications_substream`].
        id: SubstreamId,
        /// If `Ok`, contains the handshake sent back by the remote. Its interpretation is out of
        /// scope of this module.
        result: Result<Vec<u8>, (NotificationsOutErr, TNotifUd)>,
    },
    /// Remote has closed an outgoing notifications substream, meaning that it demands the closing
    /// of the substream.
    NotificationsOutCloseDemanded {
        /// Identifier of the substream. Value that was returned by
        /// [`SingleStream::open_notifications_substream`] or
        /// [`MultiStream::open_notifications_substream`].
        id: SubstreamId,
    },
    /// Remote has reset an outgoing notifications substream. The substream is instantly closed.
    NotificationsOutReset {
        /// Identifier of the substream. Value that was returned by
        /// [`SingleStream::open_notifications_substream`].
        id: SubstreamId,
        /// Value that was passed to [`SingleStream::open_notifications_substream`] or
        /// [`MultiStream::open_notifications_substream`].
        user_data: TNotifUd,
    },

    /// An outgoing ping has succeeded. This event is generated automatically over time.
    PingOutSuccess,
    /// An outgoing ping has failed. This event is generated automatically over time.
    PingOutFailed,
}

/// Configuration to turn a [`ConnectionPrototype`] into a [`SingleStream`] or [`MultiStream`].
// TODO: this struct isn't zero-cost, but making it zero-cost is kind of hard and annoying
#[derive(Debug, Clone)]
pub struct Config<TNow> {
    /// Maximum number of substreams that the remote can have simultaneously opened.
    pub max_inbound_substreams: usize,
    /// List of request-response protocols supported for incoming substreams.
    pub request_protocols: Vec<ConfigRequestResponse>,
    /// List of notifications protocols supported for incoming substreams.
    pub notifications_protocols: Vec<ConfigNotifications>,
    /// Name of the ping protocol on the network.
    pub ping_protocol: String,
    /// When to start the first outgoing ping.
    pub first_out_ping: TNow,
    /// Interval between two consecutive outgoing ping attempts.
    pub ping_interval: Duration,
    /// Time after which an outgoing ping is considered failed.
    pub ping_timeout: Duration,
    /// Entropy used for the randomness specific to this connection.
    pub randomness_seed: [u8; 32],
}

/// Configuration for a request-response protocol.
#[derive(Debug, Clone)]
pub struct ConfigRequestResponse {
    /// Name of the protocol transferred on the wire.
    pub name: String,

    /// Configuration related to sending out requests through this protocol.
    ///
    /// > **Note**: This is used even if `inbound_allowed` is `false` when performing outgoing
    /// >           requests.
    pub inbound_config: ConfigRequestResponseIn,

    pub max_response_size: usize,

    /// If true, incoming substreams are allowed to negotiate this protocol.
    pub inbound_allowed: bool,
}

/// See [`ConfigRequestResponse::inbound_config`].
#[derive(Debug, Clone)]
pub enum ConfigRequestResponseIn {
    /// Request must be completely empty, not even a length prefix.
    Empty,
    /// Request must contain a length prefix plus a potentially empty payload.
    Payload {
        /// Maximum allowed size for the payload in bytes.
        max_size: usize,
    },
}

impl ConfigRequestResponseIn {
    /// Returns the maximum allowed size of a request.
    ///
    /// Returns `0` for [`ConfigRequestResponseIn::Empty`].
    pub fn max_size(&self) -> usize {
        match self {
            ConfigRequestResponseIn::Empty => 0,
            ConfigRequestResponseIn::Payload { max_size } => *max_size,
        }
    }
}

/// Configuration for a notifications protocol.
#[derive(Debug, Clone)]
pub struct ConfigNotifications {
    /// Name of the protocol transferred on the wire.
    pub name: String,

    /// Maximum size, in bytes, of the handshake that can be received.
    pub max_handshake_size: usize,

    /// Maximum size, in bytes, of a notification that can be received.
    pub max_notification_size: usize,
}

/// Error potentially returned when starting a request.
#[derive(Debug, Clone, derive_more::Display)]
pub enum AddRequestError {
    /// Size of the request is over maximum allowed by the protocol.
    RequestTooLarge,
}
