// Copyright (c) The hasp Contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::database::ConnectionCreator;
use chrono::Local;
use color_eyre::{eyre::WrapErr, Result};
use jod_thread::JoinHandle;
use rusqlite::params;
use serde::Serialize;
use std::sync::{mpsc, Arc};

#[derive(Clone, Debug)]
pub(crate) struct EventLogger {
    // Send and receive pairs of (event name, event data).
    sender: mpsc::Sender<(&'static str, String)>,
    join_handle: Arc<JoinHandle<()>>,
}

impl EventLogger {
    pub(crate) fn new(creator: &ConnectionCreator) -> Result<Self> {
        let events_conn = creator.create_events()?;
        let (sender, receiver) = mpsc::channel();
        // Create a new thread to serialize event logging.
        let join_handle = jod_thread::Builder::new()
            .name("hasp-event-logger".to_owned())
            .spawn(move || {
                loop {
                    let (event_name, data) = match receiver.recv() {
                        Ok(event) => event,
                        Err(_) => {
                            // All senders were dropped -- shut this thread down.
                            return;
                        }
                    };
                    // TODO: begin concurrent if/when that's available?
                    // TODO: error handling for this? ignore errors for now.
                    log::debug!("recording event {}", event_name);
                    events_conn.execute(
                    "INSERT INTO journal (event_name, event_time, data) VALUES (?1, ?2, ?3)",
                    params![event_name, Local::now(), data],
                    ).expect("wat");
                }
            })
            .wrap_err("creating event logger thread failed")?;
        Ok(EventLogger {
            sender,
            join_handle: Arc::new(join_handle),
        })
    }

    pub(crate) fn log(&self, event_name: &'static str, data: &impl Serialize) {
        // This should basically never fail, but if it does, ignore the error.
        let data = match serde_json::to_string(data) {
            Ok(data) => data,
            Err(_) => return,
        };

        // Assume writing to events is lossy so ignore send errors.
        let _ = self.sender.send((event_name, data));
    }
}
