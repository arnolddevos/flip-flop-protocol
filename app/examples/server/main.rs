use circular_queue::CircularQueue;
use rand::prelude::*;
use std::{env, error::Error, net::SocketAddr, time::Duration};

use flip_flop_app::CommandRequest;
use tokio::{
    net::UdpSocket,
    sync::mpsc,
    time::{self, Instant},
};

#[path = "../common/lib.rs"]
mod common;
use crate::common::{Command, Event};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let local_addr: SocketAddr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".into())
        .parse()?;

    let socket = UdpSocket::bind(local_addr).await?;

    println!("SERVER: listening on {:?}", local_addr);

    // Generate events in the background. We simple generate timestamp
    // and send them to our main loop.

    let (event_s, mut event_r) = mpsc::channel::<Instant>(100);

    tokio::spawn(async move {
        loop {
            let event_time = Instant::now();
            let delay = { Duration::from_secs(rand::thread_rng().gen_range(0..3)) };
            if let Some(instant) = event_time.checked_add(delay) {
                time::sleep_until(instant).await;
                let _ = event_s.send(event_time).await;
            }
        }
    });

    // This size should never exceed what can be sent in one packet. If you
    // have needs that exceed this constraint then you will need to consider
    // framing.
    const MAX_DATAGRAM_SIZE: usize = 32;
    const MAX_EVENTS: usize = 10;

    let mut recv_buf = [0; MAX_DATAGRAM_SIZE];
    let mut events = CircularQueue::<(Event, u32, Instant)>::with_capacity(MAX_EVENTS);
    let mut event_offset = 0;

    loop {
        tokio::select! {
            Ok((len, remote_addr)) = socket.recv_from(&mut recv_buf) => {
                if let Ok(request) = postcard::from_bytes::<CommandRequest<Command>>(&recv_buf[..len]) {
                    println!(
                        "SERVER: {:?} command received from {:?}. Replying.",
                        request, remote_addr
                    );

                    // We optimise searching for events by going backward in our
                    // circular buffer until we find the latest event where its
                    // offset exceeds the last one observed by the client. In the
                    // case where we have no last offset expressed by the client
                    // then we provide the oldest one we have.
                    let maybe_reply = events
                            .iter()
                            .take_while(|(_, offset, _)| *offset > request.last_event_offset)
                            .last().or_else(|| events.iter().next());

                    let reply = flip_flop_app::event_reply(maybe_reply, |t|Instant::now().duration_since(t).as_secs());

                    let mut send_buf = [0; MAX_DATAGRAM_SIZE];
                    if let Ok(encoded_buf) = postcard::to_slice(&reply, &mut send_buf) {
                        let _ = socket.send_to(encoded_buf, remote_addr).await;
                        println!("SERVER: {:?} event replied to {:?}", reply, remote_addr);
                    }
                }
            }

            Some(event_instant) = event_r.recv() => {
                events.push((Event::SomeEvent, event_offset, event_instant));

                // For this example, we will reset the event offset periodically
                // so that a client can demonstrate how it forgets state.
                if rand::thread_rng().gen_range(0..10) == 0 {
                    println!("SERVER: Resetting events");
                    events.clear();
                    event_offset = 0;
                } else {
                    event_offset += 1;
                }
            }
        }
    }
}
