use std::io::{Write, Read};

fn write_yuv420_file<P: AsRef<std::path::Path>>(
    path: P,
    width: usize,
    height: usize,
    ybuf: &[u8],
    ubuf: &[u8],
    vbuf: &[u8],
) {
    let mb_width = width.div_ceil(16);
    let mb_height = height.div_ceil(16);
    let mb_count = mb_width * mb_height;
    assert_eq!(ybuf.len(), mb_count * 16 * 16);
    assert_eq!(ubuf.len(), mb_count * 8 * 8);
    assert_eq!(vbuf.len(), mb_count * 8 * 8);

    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(ybuf).unwrap();
    file.write_all(ubuf).unwrap();
    file.write_all(vbuf).unwrap();
}

pub fn read_yuv420_file<P: AsRef<std::path::Path>>(
    path: P,
    width: usize,
    height: usize,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mb_width = width.div_ceil(16);
    let mb_height = height.div_ceil(16);
    let mb_count = mb_width * mb_height;

    let mut file = std::fs::File::open(path).unwrap();

    let mut y = vec![0u8; mb_count * 16 * 16];
    let mut u = vec![0u8; mb_count * 8 * 8];
    let mut v = vec![0u8; mb_count * 8 * 8];

    file.read_exact(&mut y).unwrap();
    file.read_exact(&mut u).unwrap();
    file.read_exact(&mut v).unwrap();

    (y, u, v)
}

fn reference_test(file: &str) {
    // Prepare WebP decoder
    let contents = std::fs::read(format!("tests/images/{file}.webp")).unwrap();
    let frame = image_webp::read_image(&contents).unwrap();
    let width = frame.width as usize;
    let height = frame.height as usize;

    // Read reference YUV
    let (y, u, v) = read_yuv420_file(format!("tests/images/{file}.{width}x{height}.yuv"), width, height);

    // Compare pixels
    let num_diff_luma = frame.ybuf
        .iter()
        .zip(y.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(num_diff_luma, 0, "Luma pixel mismatch");
    let num_diff_cb = frame.ubuf
        .iter()
        .zip(u.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(num_diff_cb, 0, "Chroma blue pixel mismatch");
    let num_diff_cr = frame.vbuf
        .iter()
        .zip(v.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(num_diff_cr, 0, "Chroma red pixel mismatch");

    if num_diff_luma > 0 || num_diff_cb > 0 || num_diff_cr > 0 {
        write_yuv420_file(format!("tests/test/{file}.{width}x{height}.yuv"), width, height, &frame.ybuf, &frame.ubuf, &frame.vbuf);
    }
}

fn test_bench(file: &str) {
    // Prepare WebP decoder
    let contents = std::fs::read(format!("tests/images/{file}.webp")).unwrap();

    let start = std::time::Instant::now();

    std::iter::repeat(()).take(20).for_each(|_| {
        image_webp::read_image(&contents).unwrap();
    });

    println!("{file} took: {:?}", start.elapsed());
}

macro_rules! reftest {
    ($basename:expr, $name:expr) => {
        paste::paste! {
            #[test]
            fn [<reftest_ $basename _ $name>]() {
                reference_test(concat!(stringify!($basename), "/", stringify!($name)));
            }
        }
    };
    ($basename:expr, $name:expr, $($tail:expr),+) => {
        reftest!( $basename, $name );
        reftest!( $basename, $($tail),+ );
    }
}

macro_rules! testbench {
    ($basename:expr, $name:expr) => {
        paste::paste! {
            #[test]
            fn [<testbench_ $basename _ $name>]() {
                test_bench(concat!(stringify!($basename), "/", stringify!($name)));
            }
        }
    };
    ($basename:expr, $name:expr, $($tail:expr),+) => {
        testbench!( $basename, $name );
        testbench!( $basename, $($tail),+ );
    }
}

//reftest!(gallery1, 1, 2, 3, 4, 5, photo001, photo002);
testbench!(gallery1, 1, 2, 3, 4, 5, photo001, photo002);