use std::pin::Pin;
#[cfg(not(loom))]
use std::{
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        Condvar,
        Mutex,
    },
    thread,
};

#[cfg(loom)]
use loom::{
    sync::{
        atomic::{AtomicPtr, AtomicUsize, Ordering},
        Condvar,
        Mutex,
    },
    thread,
};

pub struct Pool {
    threads: Vec<thread::Thread>,
    inner:   Pin<Box<PoolInner>>,
}

struct PoolInner {
    func:        AtomicPtr<Box<dyn Fn(usize) + Send + Sync>>,
    max:         AtomicUsize,
    cnt:         AtomicUsize,
    finished:    AtomicUsize,
    notif_mutex: Mutex<bool>,
    notif_var:   Condvar,
    lock_mutex:  Mutex<()>,
}

impl Pool {
    pub fn new(size: usize) -> Pool {
        let threads = Vec::with_capacity(size);
        let mut pool = Pool {
            threads,
            inner: Box::pin(PoolInner {
                func:        AtomicPtr::new(std::ptr::null_mut()),
                max:         AtomicUsize::new(0),
                cnt:         AtomicUsize::new(0),
                finished:    AtomicUsize::new(0),
                notif_mutex: Mutex::new(false),
                notif_var:   Condvar::new(),
                lock_mutex:  Mutex::new(()),
            }),
        };
        let ptr = &*pool.inner.as_ref() as *const _ as usize;
        for _ in 0..size {
            pool.threads.push(
                thread::spawn(move || {
                    #[allow(invalid_reference_casting)]
                    let inner = unsafe { &mut *(ptr as *mut PoolInner) };
                    #[allow(clippy::never_loop)] // it does...
                    loop {
                        thread::park();
                        let func = inner.func.load(Ordering::SeqCst);
                        match func.is_null() {
                            false => {
                                let func =
                                    unsafe { &*func as *const Box<dyn Fn(usize) + Send + Sync> };
                                let max = inner.max.load(Ordering::SeqCst);
                                loop {
                                    let cnt = inner.cnt.fetch_add(1, Ordering::SeqCst);
                                    if cnt >= max {
                                        let old = inner.finished.fetch_add(1, Ordering::SeqCst);
                                        if old == size - 1 {
                                            *inner.notif_mutex.lock().unwrap() = true;
                                            inner.notif_var.notify_all();
                                        }
                                        break;
                                    }
                                    (unsafe { &*func })(cnt);
                                }
                            },
                            true => {
                                let old = inner.finished.fetch_add(1, Ordering::SeqCst);
                                if old == size - 1 {
                                    *inner.notif_mutex.lock().unwrap() = true;
                                    inner.notif_var.notify_all();
                                }
                                break;
                            },
                        }
                    }
                })
                .thread()
                .clone(),
            );
        }
        pool
    }

    pub fn execute(&mut self, num: usize, func: impl Fn(usize) + Send + Sync) {
        let inner = self.inner.as_mut();
        let _guard = inner.lock_mutex.lock().unwrap();
        let func = Box::new(func) as Box<dyn Fn(usize) + Send + Sync>;
        let func = Box::new(func);
        let ptr = unsafe {
            std::mem::transmute::<
                *mut std::boxed::Box<dyn Fn(usize) + Send + Sync>,
                *mut std::boxed::Box<dyn Fn(usize) + Send + Sync + 'static>,
            >(Box::into_raw(func))
        };
        inner.func.store(ptr, Ordering::SeqCst);
        inner.cnt.store(0, Ordering::SeqCst);
        inner.finished.store(0, Ordering::SeqCst);
        inner.max.store(num, Ordering::SeqCst);
        for thread in &self.threads {
            thread.unpark();
        }
        let mut notif = inner.notif_mutex.lock().unwrap();
        while !*notif {
            notif = inner.notif_var.wait(notif).unwrap();
        }
        *notif = false;
        drop(notif);
        inner.func.store(std::ptr::null_mut(), Ordering::SeqCst);
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.inner.finished.store(0, Ordering::SeqCst);
        for thread in &self.threads {
            thread.unpark();
        }
        let mut guard = self.inner.notif_mutex.lock().unwrap();
        while !*guard {
            guard = self.inner.notif_var.wait(guard).unwrap();
        }
    }
}

lazy_static::lazy_static! {
    static ref GLOBAL: std::sync::Mutex<Option<Pool>> = std::sync::Mutex::new(None);
}

pub fn execute(num: usize, func: impl Fn(usize) + Send + Sync) {
    GLOBAL
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            let size = std::env::var("IEU_NUM_THREADS")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or_else(|| {
                    std::env::var("RAYON_NUM_THREADS")
                        .ok()
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or_else(num_cpus::get)
                });
            Pool::new(size)
        })
        .execute(num, func);
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn test_pool() {
        let mut pool = Pool::new(4);
        let cnt = AtomicUsize::new(0);
        pool.execute(10, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        pool.execute(20, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        pool.execute(50, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(cnt.load(Ordering::SeqCst), 80);
        drop(pool);
    }

    #[test]
    fn test_global() {
        let cnt = AtomicUsize::new(0);
        execute(10, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        execute(20, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        execute(50, |_| {
            cnt.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(cnt.load(Ordering::SeqCst), 80);
    }

    #[test]
    fn test_par_lock() {
        let cnt = AtomicUsize::new(0);
        thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(1));
                execute(1, |_| {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    cnt.store(2, Ordering::SeqCst);
                });
            });
            s.spawn(|| {
                execute(1, |_| {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                });
            });
        });
        assert_eq!(cnt.load(Ordering::SeqCst), 2);
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    #[test]
    fn test_pool() {
        loom::model(|| {
            let mut pool = Pool::new(4);
            let cnt = AtomicUsize::new(0);
            pool.execute(10, |_| {
                cnt.fetch_add(1, Ordering::SeqCst);
            });
            pool.execute(20, |_| {
                cnt.fetch_add(1, Ordering::SeqCst);
            });
            pool.execute(50, |_| {
                cnt.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(cnt.load(Ordering::SeqCst), 80);
        });
    }
}
