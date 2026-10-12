use live_gpui_protocol::{decode, MAX_MESSAGE_BYTES, QUEUE_CAPACITY};
use rustler::{
    Atom, Binary, Encoder, Env, LocalPid, Monitor, OwnedBinary, OwnedEnv, Resource, ResourceArc,
    Term,
};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

mod atoms {
    rustler::atoms! { ok, error, busy, closed, live_gpui_native }
}

struct Bridge {
    queue: Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>,
    child: Arc<Mutex<Child>>,
    closed: Arc<AtomicBool>,
}

impl Bridge {
    fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut queue) = self.queue.0.lock() {
            queue.clear();
            self.queue.1.notify_all();
        }
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.kill();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

#[rustler::resource_impl]
impl Resource for Bridge {
    fn down(&self, _env: Env<'_>, _pid: LocalPid, _monitor: Monitor) {
        self.stop();
    }
}

fn deliver(environment: &mut OwnedEnv, owner: &LocalPid, id: u64, bytes: &[u8]) -> bool {
    environment
        .send_and_clear(owner, |env| match OwnedBinary::new(bytes.len()) {
            Some(mut binary) => {
                binary.as_mut_slice().copy_from_slice(bytes);
                (atoms::live_gpui_native(), id, binary.release(env)).encode(env)
            }
            None => (
                atoms::live_gpui_native(),
                id,
                "{\"type\":\"closed\",\"reason\":\"allocation failed\"}",
            )
                .encode(env),
        })
        .is_ok()
}

fn read_frame(reader: &mut impl BufRead, buffer: &mut Vec<u8>) -> std::io::Result<bool> {
    buffer.clear();
    let count = reader
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', buffer)?;
    if count == 0 {
        return Ok(false);
    }
    // The bound includes the delimiter; exactly MAX_MESSAGE_BYTES of payload is valid.
    if buffer.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unterminated or oversized native frame",
        ));
    }
    buffer.pop();
    Ok(true)
}

#[rustler::nif(schedule = "DirtyIo")]
fn open(
    env: Env<'_>,
    executable: String,
    owner: LocalPid,
    id: u64,
) -> Result<ResourceArc<Bridge>, String> {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdin = child.stdin.take().ok_or("host stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("host stdout unavailable")?;
    let child = Arc::new(Mutex::new(child));
    let closed = Arc::new(AtomicBool::new(false));
    let queue = Arc::new((
        Mutex::new(VecDeque::<Vec<u8>>::with_capacity(QUEUE_CAPACITY)),
        Condvar::new(),
    ));
    let writer_queue = queue.clone();
    let writer_closed = closed.clone();
    let writer_child = child.clone();
    std::thread::Builder::new()
        .name("live-gpui-writer".into())
        .spawn(move || {
            loop {
                let message = {
                    let Ok(mut queue) = writer_queue.0.lock() else {
                        break;
                    };
                    while queue.is_empty() && !writer_closed.load(Ordering::Acquire) {
                        let Ok(next) = writer_queue.1.wait(queue) else {
                            return;
                        };
                        queue = next;
                    }
                    if writer_closed.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(message) = queue.pop_front() else {
                        break;
                    };
                    message
                };
                if stdin
                    .write_all(&message)
                    .and_then(|_| stdin.write_all(b"\n"))
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    writer_closed.store(true, Ordering::Release);
                    if let Ok(mut child) = writer_child.lock() {
                        let _ = child.kill();
                    }
                    break;
                }
            }
            // Closing stdin causes the host's bounded reader to close its main-thread loop.
        })
        .map_err(|error| {
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
            error.to_string()
        })?;
    let reader_child = child.clone();
    let reader_closed = closed.clone();
    let reader_queue = queue.clone();
    std::thread::Builder::new()
        .name("live-gpui-events".into())
        .spawn(move || {
            let mut environment = OwnedEnv::new();
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            loop {
                match read_frame(&mut reader, &mut line) {
                    Ok(false) | Err(_) => break,
                    Ok(true) => {
                        if !deliver(&mut environment, &owner, id, &line) {
                            break;
                        }
                    }
                }
            }
            reader_closed.store(true, Ordering::Release);
            if let Ok(_queue) = reader_queue.0.lock() {
                reader_queue.1.notify_all();
            }
            if let Ok(mut child) = reader_child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            deliver(
                &mut environment,
                &owner,
                id,
                b"{\"type\":\"closed\",\"reason\":\"host exited\"}",
            );
        })
        .map_err(|error| {
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
            error.to_string()
        })?;
    let resource = ResourceArc::new(Bridge {
        queue,
        child,
        closed,
    });
    if resource.monitor(Some(env), &owner).is_none() {
        resource.stop();
        return Err("cannot monitor host owner".into());
    }
    Ok(resource)
}

#[rustler::nif]
fn submit<'a>(env: Env<'a>, resource: ResourceArc<Bridge>, message: Binary<'a>) -> Term<'a> {
    if resource.closed.load(Ordering::Acquire) {
        return (atoms::error(), atoms::closed()).encode(env);
    }
    if let Err(reason) = decode(message.as_slice()) {
        return (atoms::error(), reason).encode(env);
    }
    let Ok(mut queue) = resource.queue.0.try_lock() else {
        return (atoms::error(), atoms::busy()).encode(env);
    };
    if resource.closed.load(Ordering::Acquire) {
        return (atoms::error(), atoms::closed()).encode(env);
    }
    if queue.len() == QUEUE_CAPACITY {
        return (atoms::error(), atoms::busy()).encode(env);
    }
    queue.push_back(message.as_slice().to_vec());
    resource.queue.1.notify_one();
    atoms::ok().encode(env)
}

#[rustler::nif(schedule = "DirtyIo")]
fn close(resource: ResourceArc<Bridge>) -> Atom {
    resource.stop();
    atoms::ok()
}

rustler::init!("Elixir.LiveGPUI.Native");

#[cfg(test)]
mod tests {
    use super::{read_frame, MAX_MESSAGE_BYTES};
    use std::io::{Cursor, ErrorKind};

    #[test]
    fn complete_frames_at_the_payload_limit_are_not_treated_as_host_exit() {
        for size in [MAX_MESSAGE_BYTES - 1, MAX_MESSAGE_BYTES] {
            let mut bytes = vec![b'x'; size];
            bytes.push(b'\n');
            let mut reader = Cursor::new(bytes);
            let mut frame = Vec::new();
            assert!(read_frame(&mut reader, &mut frame).unwrap());
            assert!(!read_frame(&mut reader, &mut frame).unwrap());
        }
    }

    #[test]
    fn oversized_or_incomplete_frames_fail_instead_of_being_delivered_partially() {
        let mut oversized = vec![b'x'; MAX_MESSAGE_BYTES + 1];
        oversized.push(b'\n');
        for bytes in [oversized, b"incomplete".to_vec()] {
            let error = read_frame(&mut Cursor::new(bytes), &mut Vec::new()).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData);
        }
    }
}
