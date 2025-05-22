# ieu

`ieu` is a package for simple, minimal overhead (a handful of atomics at startup, one for each iteration) parallel execution of a function over a series of indices. It is designed for use in situations with few calls to `execute` on the thread pool (due to the relatively high overhead compared to other solutions, `ieu` doesn't use a spin-lock so takes 10-15 microseconds to wake each thread) but each call to execute is expensive so can benefit from the low per iteration overhead.

## Usage

`ieu` provides a global thread pool constructed on demand using the environment variables `IEU_NUM_THREADS`, `RAYON_NUM_THREADS` (if `IEU_NUM_THREADS` is not set), or the number of CPU threads on teh system if neither is set.

```rust
// run on the global thread
ieu::execute(
    3,
    |i| {
        // i ranges 0..3
        println!("Hello from iteration {}", i);
    },
);
```

`ieu` also allows you to create custom thread pools with varying numbers of threads. This is useful for testing or if you want to use a different number of threads for different parts of your program.

```rust
// create a thread pool with 4 threads
let mut pool = ieu::Pool::new(4);

// run on the custom thread pool
pool.execute(
    3,
    |i| {
        // i ranges 0..3
        println!("Hello from iteration {}", i);
    },
);
```

## Important Note
`ieu` is not a general purpose thread pool like `rayon`, it's designed to run a single expensive task at a time, not many tasks from many sources. Each call to `execute` will block the current thread and lock the thread pool until the task is complete.
