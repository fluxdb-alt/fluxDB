// 独立于 GPUI/libssh2，验证本地 TCP 读取消在目标操作系统上的行为。
use std::io::{self, Read};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

fn probe(nonblocking: bool) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (socket, _) = listener.accept().unwrap();
    socket.set_nonblocking(nonblocking).unwrap();
    // 兜底让诊断本身有界；生产代码不依赖此超时。
    socket
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let mut reader = socket.try_clone().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        let mut byte = [0];
        loop {
            if nonblocking && worker_stop.load(Ordering::Acquire) {
                break;
            }
            match reader.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if nonblocking && error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => {
                    println!("read result: {error}");
                    break;
                }
            }
        }
        let _ = done_tx.send(());
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    stop.store(true, Ordering::Release);
    println!(
        "nonblocking={nonblocking}, shutdown={:?}",
        socket.shutdown(Shutdown::Both)
    );
    let stopped = done_rx.recv_timeout(Duration::from_secs(1)).is_ok();
    println!("nonblocking={nonblocking}, stopped_within_1s={stopped}");
    // 主动关闭对端，避免阻塞模式的诊断留下线程。
    drop(client);
    worker.join().unwrap();
    if nonblocking {
        assert!(stopped, "nonblocking reader ignored cancellation");
    }
}

fn main() {
    // 整体超时兜底；不把主线程栈冒充为被阻塞 worker 的调用栈。
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        if done_rx.recv_timeout(Duration::from_secs(15)).is_err() {
            eprintln!("socket probe exceeded 15s; see last reported stage");
            std::process::exit(124);
        }
    });
    println!("platform={}", std::env::consts::OS);
    probe(false);
    probe(true);
    let _ = done_tx.send(());
}
