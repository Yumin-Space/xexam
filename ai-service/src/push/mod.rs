//! The web push courier (v3.1): a stateless task that delivers the messages the main backend prepared.
//!
//! The backend owns subscriptions, the queue, the texts and the encryption (RFC 8291) and the VAPID signature
//! (RFC 8292); it has no internet access. This service long-polls `/internal/ai/push/next`, POSTs each job to the push
//! service of the browser vendor and reports the outcome with `/internal/ai/push/result`. It never sees the keys of a
//! device, the VAPID private key or the plain text, so it can neither read nor forge a notification. It runs whether
//! AI+ is on or off and never touches the AI usage rows or quotas.
//!
//! * `allow`  - the endpoint allow-list (the same rules and test table as the backend's)
//! * `client` - the HTTP call, the DNS guard and the mapping of an answer to an outcome
//! * `worker` - the background task that serves the backend's queue
pub mod allow;
pub mod client;
pub mod worker;
