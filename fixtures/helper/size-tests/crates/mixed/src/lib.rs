mod api;

pub fn run() -> i32 {
    api::answer()
}

#[cfg(test)]
mod tests;
