use std::fs::create_dir_all;
use std::io::{Cursor, Write};
use std::path::PathBuf;

// Write images to `out/` directory on test failure - useful for diffing with reference images.
const WRITE_IMAGES_ON_FAILURE: bool = false;

fn save_image(
    data: &[u8],
    file: &str,
    i: Option<u32>,
    has_alpha: bool,
    width: usize,
    height: usize,
) {
    if !WRITE_IMAGES_ON_FAILURE {
        return;
    }

    let path = PathBuf::from(match i {
        Some(i) => format!("tests/out/{file}-{i}.png"),
        None => format!("tests/out/{file}.png"),
    });

    let directory = path.parent().unwrap();
    if !directory.exists() {
        create_dir_all(directory).unwrap();
    }

    let mut f = std::fs::File::create(path).unwrap();

    let mut encoder = png::Encoder::new(&mut f, width as u32, height as u32);
    if has_alpha {
        encoder.set_color(png::ColorType::Rgba);
    } else {
        encoder.set_color(png::ColorType::Rgb);
    }
    encoder
        .write_header()
        .unwrap()
        .write_image_data(data)
        .unwrap();
    f.flush().unwrap();
}

fn reference_test(file: &str) {
    // Prepare WebP decoder
    let contents = std::fs::read(format!("tests/images/{file}.webp")).unwrap();
    let mut decoder = image_webp::WebPDecoder::new(contents).unwrap();
    let (width, height) = decoder.dimensions();

    // Decode reference PNG
    let reference_file = file;
    let reference_path = format!("tests/reference/{reference_file}.png");
    let reference_contents = std::fs::read(reference_path).unwrap();
    let mut reference_decoder = png::Decoder::new(Cursor::new(reference_contents))
        .read_info()
        .unwrap();
    assert_eq!(reference_decoder.info().bit_depth, png::BitDepth::Eight);
    let mut reference_data = vec![0; reference_decoder.output_buffer_size()];
    reference_decoder.next_frame(&mut reference_data).unwrap();

    // Compare metadata
    assert_eq!(width, reference_decoder.info().width as usize);
    assert_eq!(height, reference_decoder.info().height as usize);

    // Decode WebP
    let bytes_per_pixel = 3;
    let mut data = vec![0; width as usize * height as usize * bytes_per_pixel];
    decoder.read_image(&mut data).unwrap();

    // Compare pixels
    // NOTE: WebP lossy images are stored in YUV format. The conversion to RGB is not precisely
    // defined, but we currently attempt to match the dwebp's default conversion option.
    let num_bytes_different = data
        .iter()
        .zip(reference_data.iter())
        .filter(|(a, b)| a != b)
        .count();
    if num_bytes_different > 0 {
        save_image(&data, file, None, false, width, height);
    }
    assert_eq!(num_bytes_different, 0, "Pixel mismatch");
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

reftest!(gallery1, 1, 2, 3, 4, 5);
