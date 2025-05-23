use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use kernel_guard::{BaseGuard, IrqSave};

#[derive(Debug)]
pub struct YieldFuture {
    _has_polled: bool,
    _irq_state: <IrqSave as BaseGuard>::State,
}

impl YieldFuture {
    pub fn new() -> Self {
        // 这里获取中断状态，并且关中断
        let _irq_state = IrqSave::acquire();
        Self {
            _has_polled: false,
            _irq_state,
        }
    }
}
///yieldnow
pub fn  yield_now() -> YieldFuture {
    YieldFuture::new()
}
impl Future for YieldFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            let this = self.get_mut();
            if this._has_polled {
                // 恢复原来的中断状态
                IrqSave::release(this._irq_state);
                Poll::Ready(())
            } else {
                trace!("yield_now");
                //第一次执行到这
                this._has_polled = true;
                Poll::Pending
            }
        }
    }
}