use std::fs;
use zengif::{Decoder, Limits, Unstoppable};

fn main() {
    let test_dir = "/tmp/gif-testset";
    for entry in fs::read_dir(test_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "gif").unwrap_or(false) {
            let data = fs::read(&path).unwrap();
            let cursor = std::io::Cursor::new(&data);
            let mut decoder = Decoder::new(cursor, Limits::none(), Unstoppable).unwrap();
            
            let mut count = 0;
            while let Ok(Some(_frame)) = decoder.next_frame() {
                count += 1;
            }
            println!("{}: {} frames", path.file_name().unwrap().to_string_lossy(), count);
        }
    }
}
