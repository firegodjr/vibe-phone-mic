//! Tiny CPU rasterizer for egui output, so the UI can be rendered to a PNG
//! fully offscreen (no window, no GPU, no display server).

use eframe::egui::{self, epaint, Color32, ColorImage, TextureId};
use std::collections::HashMap;

/// Holds textures uploaded by egui between frames.
#[derive(Default)]
pub struct SoftRenderer {
    textures: HashMap<TextureId, ColorImage>,
}

impl SoftRenderer {
    pub fn new() -> SoftRenderer {
        SoftRenderer::default()
    }

    /// Apply texture uploads/frees from a frame's output.
    pub fn update_textures(&mut self, delta: &mut epaint::textures::TexturesDelta) {
        for (id, deltas) in delta.set.drain() {
            for d in deltas {
                let egui::ImageData::Color(img) = &d.image;
                match d.pos {
                    None => {
                        self.textures.insert(id, (**img).clone());
                    }
                    Some([x0, y0]) => {
                        if let Some(tex) = self.textures.get_mut(&id) {
                            for y in 0..img.size[1] {
                                for x in 0..img.size[0] {
                                    let (tx, ty) = (x0 + x, y0 + y);
                                    if tx < tex.size[0] && ty < tex.size[1] {
                                        tex.pixels[ty * tex.size[0] + tx] = img.pixels[y * img.size[0] + x];
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for id in delta.free.drain() {
            self.textures.remove(&id);
        }
    }

    /// Rasterize primitives into an image of `size` physical pixels.
    pub fn paint(&self, prims: &[epaint::ClippedPrimitive], size: [usize; 2], ppp: f32, clear: Color32) -> ColorImage {
        let mut img = ColorImage::filled(size, clear);
        for cp in prims {
            let epaint::Primitive::Mesh(mesh) = &cp.primitive else { continue };
            let tex = self.textures.get(&mesh.texture_id);
            let clip = [
                (cp.clip_rect.min.x * ppp).round().max(0.0) as i64,
                (cp.clip_rect.min.y * ppp).round().max(0.0) as i64,
                ((cp.clip_rect.max.x * ppp).round() as i64).min(size[0] as i64),
                ((cp.clip_rect.max.y * ppp).round() as i64).min(size[1] as i64),
            ];
            for tri in mesh.indices.chunks_exact(3) {
                let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
                raster_triangle(&mut img, v, ppp, clip, tex);
            }
        }
        img
    }
}

fn sample(tex: &ColorImage, u: f32, v: f32) -> [f32; 4] {
    // Bilinear, clamp-to-edge.
    let (w, h) = (tex.size[0] as f32, tex.size[1] as f32);
    let x = (u * w - 0.5).clamp(0.0, w - 1.0);
    let y = (v * h - 0.5).clamp(0.0, h - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(tex.size[0] - 1), (y0 + 1).min(tex.size[1] - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize| {
        let c = tex.pixels[y * tex.size[0] + x];
        [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        out[i] = (top + (bot - top) * fy) / 255.0;
    }
    out
}

/// Edge function for a→b at p, computed with canonically ordered endpoints so that
/// the shared edge of two adjacent triangles gives exactly opposite values.
fn edge(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    let (lo, hi, sign) = if a <= b { (a, b, 1.0) } else { (b, a, -1.0) };
    sign * ((hi.0 - lo.0) * (p.1 - lo.1) - (hi.1 - lo.1) * (p.0 - lo.0))
}

/// Tie-break for pixels exactly on an edge (top-left style rule) so each pixel is
/// covered by exactly one of two adjacent triangles: no double blending, no seams.
fn owns_edge(a: (f64, f64), b: (f64, f64)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    dy > 0.0 || (dy == 0.0 && dx < 0.0)
}

fn raster_triangle(img: &mut ColorImage, v: [&epaint::Vertex; 3], ppp: f32, clip: [i64; 4], tex: Option<&ColorImage>) {
    let pt = |v: &epaint::Vertex| ((v.pos.x * ppp) as f64, (v.pos.y * ppp) as f64);
    let (p0, mut p1, mut p2) = (pt(v[0]), pt(v[1]), pt(v[2]));
    let mut vv = v;
    let mut area = edge(p0, p1, p2);
    if area == 0.0 {
        return;
    }
    if area < 0.0 {
        // Normalise orientation so "inside" is positive for all edges.
        std::mem::swap(&mut p1, &mut p2);
        vv.swap(1, 2);
        area = -area;
    }
    let xs = [p0.0, p1.0, p2.0];
    let ys = [p0.1, p1.1, p2.1];
    let minx = (xs.iter().cloned().fold(f64::MAX, f64::min).floor() as i64).max(clip[0]);
    let maxx = (xs.iter().cloned().fold(f64::MIN, f64::max).ceil() as i64).min(clip[2]);
    let miny = (ys.iter().cloned().fold(f64::MAX, f64::min).floor() as i64).max(clip[1]);
    let maxy = (ys.iter().cloned().fold(f64::MIN, f64::max).ceil() as i64).min(clip[3]);
    let col: Vec<[f32; 4]> = vv
        .iter()
        .map(|v| [v.color.r() as f32 / 255.0, v.color.g() as f32 / 255.0, v.color.b() as f32 / 255.0, v.color.a() as f32 / 255.0])
        .collect();
    let (t0, t1, t2) = (owns_edge(p1, p2), owns_edge(p2, p0), owns_edge(p0, p1));
    let inside = |e: f64, owns: bool| e > 0.0 || (e == 0.0 && owns);
    let w = img.size[0];
    for y in miny..maxy {
        for x in minx..maxx {
            let p = (x as f64 + 0.5, y as f64 + 0.5);
            let e0 = edge(p1, p2, p);
            let e1 = edge(p2, p0, p);
            let e2 = edge(p0, p1, p);
            if !(inside(e0, t0) && inside(e1, t1) && inside(e2, t2)) {
                continue;
            }
            let (w0, w1, w2) = ((e0 / area) as f32, (e1 / area) as f32, (e2 / area) as f32);
            let mut c = [0.0f32; 4];
            for i in 0..4 {
                c[i] = col[0][i] * w0 + col[1][i] * w1 + col[2][i] * w2;
            }
            if let Some(t) = tex {
                let u = vv[0].uv.x * w0 + vv[1].uv.x * w1 + vv[2].uv.x * w2;
                let q = vv[0].uv.y * w0 + vv[1].uv.y * w1 + vv[2].uv.y * w2;
                let s = sample(t, u, q);
                for i in 0..4 {
                    c[i] *= s[i];
                }
            }
            // Premultiplied "over" blend in gamma space (what egui expects).
            let idx = y as usize * w + x as usize;
            let d = img.pixels[idx];
            let inv = 1.0 - c[3].clamp(0.0, 1.0);
            let blend = |s: f32, d: u8| ((s * 255.0) + d as f32 * inv).round().clamp(0.0, 255.0) as u8;
            img.pixels[idx] = Color32::from_rgba_premultiplied(blend(c[0], d.r()), blend(c[1], d.g()), blend(c[2], d.b()), blend(c[3], d.a()));
        }
    }
}

/// Run one UI pass of `draw` at `size_pts` logical size; rasterize it when `paint` is set.
pub fn render_ui(
    ctx: &egui::Context,
    renderer: &mut SoftRenderer,
    size_pts: [f32; 2],
    ppp: f32,
    clear: Color32,
    paint: bool,
    mut draw: impl FnMut(&mut egui::Ui),
) -> Option<ColorImage> {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size_pts[0], size_pts[1]))),
        ..Default::default()
    };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    let mut out = ctx.run_ui(input, |ui| draw(ui));
    renderer.update_textures(&mut out.textures_delta);
    if !paint {
        return None;
    }
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    let size = [(size_pts[0] * ppp).round() as usize, (size_pts[1] * ppp).round() as usize];
    Some(renderer.paint(&prims, size, out.pixels_per_point, clear))
}
