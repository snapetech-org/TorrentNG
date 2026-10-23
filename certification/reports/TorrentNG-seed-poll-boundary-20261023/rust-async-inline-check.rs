use std::{future::Future, task::{Context, Poll, Waker}};
#[inline(never)]
async fn coroutine() -> u32 { std::hint::black_box(123) }
#[unsafe(no_mangle)]
pub fn caller() -> Poll<u32> {
    std::pin::pin!(coroutine()).as_mut().poll(&mut Context::from_waker(Waker::noop()))
}
