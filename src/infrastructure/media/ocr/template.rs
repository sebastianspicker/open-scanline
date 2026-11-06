//! Dependency-free connected-component 5x7 template recognizer.
use crate::domain::image::{image_to_luma8, ImageBuffer};
use crate::error::Result;

const T: &[(char, &str)] = &[
    ('0', " ### #   ##   ##   ##   ##   # ### "),
    ('1', "  #   ##    #    #    #    #   ### "),
    ('2', " ### #   #    #   #   #   #   #####"),
    ('3', " ### #   #    # ###     ##   # ### "),
    ('4', "   #   ##  # # #  # #####   #    # "),
    ('5', "######    ####     #    ##   # ### "),
    ('6', " ### #    #### #   ##   ##   # ### "),
    ('7', "#####    #   #   #   #   #   #   # "),
    ('8', " ### #   ##   # ### #   ##   # ### "),
    ('9', " ### #   ##   # ####    ##   # ### "),
    ('A', " ### #   ##   #######   ##   ##   #"),
    ('B', "#### #   ##   ###### #   ##   #### "),
    ('C', " ### #   ##    #    #    #   # ### "),
    ('E', "######    #    #### #    #    #####"),
    ('F', "######    #    #### #    #    #    "),
    ('H', "#   ##   ##   #######   ##   ##   #"),
    ('I', " ###   #    #    #    #    #   ### "),
    ('K', "#   ##  # # #  ##   # #  #  # #   #"),
    ('L', "#    #    #    #    #    #    #####"),
    ('M', "#   ### ### # ## # ##   ##   ##   #"),
    ('N', "#   ###  ## # ##  ###   ##   ##   #"),
    ('O', " ### #   ##   ##   ##   ##   # ### "),
    ('P', "#### #   ##   ###### #    #    #   "),
    ('R', "#### #   ##   ##### # #  #  # #   #"),
    ('S', " ### #   ##     ###     ##   # ### "),
    ('T', "#####  #    #    #    #    #    #  "),
    ('U', "#   ##   ##   ##   ##   ##   # ### "),
    ('V', "#   ##   ##   ##   ##   ## # #  #  "),
    ('W', "#   ##   ##   ## # ## # ### ###   #"),
    ('X', "#   ##   # # #   #   # # #   ##   #"),
    ('Y', "#   ##   # # #   #    #    #    #  "),
    ('Z', "#####    #   #   #   #   #    #####"),
    ('.', "                                #  "),
    (',', "                           #   #   "),
    ('-', "               #####               "),
    (':', "            #         #            "),
    (';', "            #         #    #   #   "),
    ('!', "  #    #    #    #    #         #  "),
];
#[derive(Clone, Copy)]
struct C {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}
impl C {
    fn w(self) -> usize {
        self.x1 - self.x0
    }
    fn h(self) -> usize {
        self.y1 - self.y0
    }
}
fn parts(b: &[u8], w: usize, h: usize) -> Vec<C> {
    let mut v = vec![false; b.len()];
    let mut o = vec![];
    for i in 0..b.len() {
        if b[i] == 0 || v[i] {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (i % w, i % w, i / w, i / w);
        let mut q = vec![i];
        v[i] = true;
        while let Some(n) = q.pop() {
            let (x, y) = (n % w, n / w);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            for z in [
                (x + 1 < w).then(|| n + 1),
                (x > 0).then(|| n - 1),
                (y + 1 < h).then(|| n + w),
                (y > 0).then(|| n - w),
            ]
            .into_iter()
            .flatten()
            {
                enqueue_ink_neighbor(b, &mut v, &mut q, z);
            }
        }
        let c = C {
            x0,
            y0,
            x1: x1 + 1,
            y1: y1 + 1,
        };
        if (3..=(h / 2).max(3)).contains(&c.h()) && (1..=(w * 2 / 5).max(1)).contains(&c.w()) {
            o.push(c)
        }
    }
    o
}
fn enqueue_ink_neighbor(b: &[u8], visited: &mut [bool], queue: &mut Vec<usize>, z: usize) {
    if b[z] != 0 && !visited[z] {
        visited[z] = true;
        queue.push(z);
    }
}
fn glyph(b: &[u8], w: usize, c: C) -> (char, f64) {
    let mut g = [false; 35];
    for (i, cell) in g.iter_mut().enumerate() {
        let (a, z) = (i % 5, i / 5);
        let (y0, y1) = (
            c.y0 + z * c.h() / 7,
            (c.y0 + (z + 1) * c.h() / 7).max(c.y0 + z * c.h() / 7 + 1),
        );
        let (x0, x1) = (
            c.x0 + a * c.w() / 5,
            (c.x0 + (a + 1) * c.w() / 5).max(c.x0 + a * c.w() / 5 + 1),
        );
        *cell = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| b[y * w + x] as usize))
            .sum::<usize>()
            * 2
            > (y1 - y0) * (x1 - x0)
    }
    T.iter()
        .map(|(c, t)| {
            (
                *c,
                t.bytes().zip(g).filter(|(e, a)| (*e == b'#') == *a).count() as f64 / 35.,
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}
/// Groups components into lines (centre inside the line's vertical span), each sorted by x.
fn lines(mut c: Vec<C>) -> Vec<Vec<C>> {
    c.sort_by_key(|x| (x.y0, x.x0));
    let mut out: Vec<(usize, usize, Vec<C>)> = vec![];
    for x in c {
        let mid = (x.y0 + x.y1) / 2;
        match out.last_mut().filter(|l| (l.0..l.1).contains(&mid)) {
            Some(l) => {
                l.0 = l.0.min(x.y0);
                l.1 = l.1.max(x.y1);
                l.2.push(x)
            }
            None => out.push((x.y0, x.y1, vec![x])),
        }
    }
    out.into_iter()
        .map(|mut l| {
            l.2.sort_by_key(|x| x.x0);
            l.2
        })
        .collect()
}
pub fn recognize(image: &ImageBuffer) -> Result<super::OcrResult> {
    let (w, h, g) = image_to_luma8(image)?;
    let b = g.into_iter().map(|x| u8::from(x < 128)).collect::<Vec<_>>();
    let c = parts(&b, w as usize, h as usize);
    if c.is_empty() {
        return Ok(out(super::NO_TEXT_PLACEHOLDER.into(), 0.));
    }
    let mut s = String::new();
    let mut q = vec![];
    for (i, line) in lines(c).into_iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        for (j, x) in line.iter().enumerate() {
            // A gap wider than half a glyph is a word space.
            if j > 0 && x.x0.saturating_sub(line[j - 1].x1) * 2 > line[j - 1].w() {
                s.push(' ');
            }
            let (a, z) = glyph(&b, w as usize, *x);
            s.push(a);
            q.push(z)
        }
    }
    Ok(out(s, q.iter().sum::<f64>() / q.len() as f64))
}
fn out(text: String, confidence: f64) -> super::OcrResult {
    super::OcrResult {
        text,
        engine: super::OFFLINE_OCR_ENGINE.into(),
        confidence,
        language: "und".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image::PixelFormat;

    /// Draws glyph rows (`#` = ink) at 4 px per cell at (ox, oy) on a white canvas.
    fn draw(canvas: &mut [u8], w: usize, ox: usize, oy: usize, t: &str) {
        for (i, ch) in t.bytes().enumerate() {
            if ch == b'#' {
                // 5 px blocks on a 4 px pitch so diagonal strokes stay 4-connected.
                for y in 0..5 {
                    for x in 0..5 {
                        canvas[(oy + i / 5 * 4 + y) * w + ox + i % 5 * 4 + x] = 0;
                    }
                }
            }
        }
    }

    fn text(w: usize, h: usize, items: &[(usize, usize, char)]) -> String {
        let mut canvas = vec![255u8; w * h];
        for (x, y, c) in items {
            draw(
                &mut canvas,
                w,
                *x,
                *y,
                T.iter().find(|t| t.0 == *c).unwrap().1,
            );
        }
        let img = ImageBuffer::new(w as u32, h as u32, PixelFormat::Gray8, canvas).unwrap();
        recognize(&img).unwrap().text
    }

    #[test]
    fn every_template_has_35_cells() {
        for (c, t) in T {
            assert_eq!(t.len(), 35, "glyph {c}");
        }
    }

    #[test]
    fn redrawn_glyphs_recognise_themselves() {
        // ':' and ';' are several components, which the bbox recognizer cannot read.
        for c in ['2', '5', 'R', 'W'] {
            assert_eq!(text(80, 80, &[(20, 20, c)]), c.to_string());
        }
    }

    #[test]
    fn same_line_reads_left_to_right_despite_top_offset() {
        assert_eq!(text(160, 100, &[(20, 21, 'H'), (44, 20, 'L')]), "HL");
        assert_eq!(text(160, 100, &[(44, 20, 'L'), (20, 21, 'H')]), "HL");
    }

    #[test]
    fn lines_and_spaces_are_emitted() {
        assert_eq!(
            text(
                200,
                200,
                &[(20, 20, 'H'), (44, 20, 'H'), (100, 20, 'H'), (20, 100, 'T')]
            ),
            "HH H\nT"
        );
    }
}
