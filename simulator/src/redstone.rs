use std::sync::{Arc, Mutex};

const SIDE_NAMES: [&str; 6] = ["DOWN", "UP", "FRONT", "BACK", "LEFT", "RIGHT"];

#[derive(Clone)]
pub struct RedstoneState {
    inner: Arc<Mutex<RedstoneInner>>,
}

struct RedstoneInner {
    outputs: [i32; 6],
    inputs: [i32; 6],
}

impl RedstoneState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RedstoneInner {
                outputs: [0; 6],
                inputs: [0; 6],
            })),
        }
    }

    pub fn set_output(&self, side: i32, power: i32) -> i32 {
        if side < 0 || side > 5 {
            return -1;
        }
        let power = power.clamp(0, 15);
        let mut inner = self.inner.lock().unwrap();
        inner.outputs[side as usize] = power;
        eprintln!("[Redstone] {} output = {}", SIDE_NAMES[side as usize], power);
        0
    }

    pub fn get_input(&self, side: i32) -> i32 {
        if side < 0 || side > 5 {
            return 0;
        }
        self.inner.lock().unwrap().inputs[side as usize]
    }

    pub fn get_all_inputs(&self) -> [i32; 6] {
        self.inner.lock().unwrap().inputs
    }

    pub fn set_input(&self, side: i32, power: i32) -> [i32; 6] {
        if side < 0 || side > 5 {
            return self.get_all_inputs();
        }
        let power = power.clamp(0, 15);
        let mut inner = self.inner.lock().unwrap();
        let old = inner.inputs;
        inner.inputs[side as usize] = power;
        old
    }

    pub fn get_inputs_for_json(&self) -> ([i32; 6], [i32; 6]) {
        let inner = self.inner.lock().unwrap();
        (inner.inputs, inner.inputs) // caller handles old vs new
    }
}
