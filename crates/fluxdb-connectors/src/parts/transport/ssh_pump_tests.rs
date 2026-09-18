// 不依赖外部 SSH 服务，验证背压及异常下的字节完整性和取消。
mod ssh_pump_tests {
    use super::pump_ssh_bytes;
    use std::io::{self, Cursor, Read, Write};

    struct ShortWriter {
        bytes: Vec<u8>,
        calls: usize,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.calls += 1;
            match self.calls % 3 {
                1 => Err(io::ErrorKind::WouldBlock.into()),
                2 => Err(io::ErrorKind::Interrupted.into()),
                _ => {
                    let n = bytes.len().min(4096);
                    self.bytes.extend_from_slice(&bytes[..n]);
                    Ok(n)
                }
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn backpressure_and_short_writes_preserve_all_bytes() {
        let bytes: Vec<u8> = (0..70_000).map(|n| (n % 251) as u8).collect();
        let mut output = ShortWriter {
            bytes: vec![],
            calls: 0,
        };
        pump_ssh_bytes(&mut Cursor::new(&bytes), &mut output, || false).unwrap();
        assert_eq!(output.bytes, bytes);
    }

    struct CancelOnIo<'a>(&'a std::cell::Cell<bool>);
    impl Read for CancelOnIo<'_> {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            assert!(!self.0.replace(true), "取消后不应继续读取");
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    impl Write for CancelOnIo<'_> {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            assert!(!self.0.replace(true), "取消后不应继续写入");
            Err(io::ErrorKind::WouldBlock.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn cancellation_during_read_and_write_is_observed() {
        let canceled = std::cell::Cell::new(false);
        pump_ssh_bytes(&mut CancelOnIo(&canceled), &mut io::sink(), || {
            canceled.get()
        })
        .unwrap();
        assert!(canceled.replace(false));
        pump_ssh_bytes(
            &mut Cursor::new(b"pending"),
            &mut CancelOnIo(&canceled),
            || canceled.get(),
        )
        .unwrap();
        assert!(canceled.get());
    }

    #[test]
    fn zero_write_is_an_error_not_success() {
        let error =
            pump_ssh_bytes(&mut Cursor::new(b"pending"), &mut &mut [][..], || false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    }
}
