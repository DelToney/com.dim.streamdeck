use skia_safe::{
    image_filters, FilterMode, Image, ImageFilter, MipmapMode, Rect, SamplingOptions, TileMode,
};

pub fn scale_image(image: Image, width: f32, height: f32) -> Image {
    let scaling = image_filters::image(
        image.clone(),
        Some(&Rect::new(
            0.0,
            0.0,
            image.width() as f32,
            image.height() as f32,
        )),
        Some(&Rect::new(0.0, 0.0, width, height)),
        Some(SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)),
    );
    return apply_filter(image, scaling);
}

fn apply_filter(image: Image, filter: Option<ImageFilter>) -> Image {
    if let Some(filter) = filter {
        if let Some((new_image, _, _)) =
            image.new_with_filter(&filter, image.bounds(), image.bounds())
        {
            return new_image;
        }
    }
    return image;
}

pub fn blur_image(image: Image, sigma: f32) -> Image {
    let filter = image_filters::blur((sigma, sigma), TileMode::Mirror, None, image.bounds());
    return apply_filter(image.clone(), filter);
}
