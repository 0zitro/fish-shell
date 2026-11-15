// OKLCH states a colour as a lightness, a chroma and a hue.  The page's colour picker slices it
// one channel at a time: the lightness against the chroma with the hue held, the hue against the
// chroma with the lightness held, and the hue against the lightness with the chroma held.  The
// colour the cursor lands on is stated as the hex a theme value is written as.  The conversions
// are the OKLab matrices with the sRGB transfer function on either side of them; a chroma the
// screen cannot show is brought down until it can, since a theme value is a colour a terminal
// shows.
//
// A card's plane is drawn on the GPU: one fragment shader states the colour of every pixel of the
// plane, so a card costs a single draw call however large it is.  The matrices below are the ones
// the conversions here are stated with and the ones the shader is built from, so there is one
// statement of them.

// The OKLab matrices: linear sRGB to the cone response, that response's axes, and the inverse of
// each.
const oklch_lms_from_linear = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];

const oklch_linear_from_lms = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.707614701],
];

const oklch_lab_from_lms = [
    [0.2104542553, 0.793617785, -0.0040720468],
    [1.9779984951, -2.428592205, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.808675766],
];

// The lightness, and the chroma turned by the hue, give the cone response's axes their third power
// back: the first column is the lightness' own coefficient in each.
const oklch_lms_from_lab = [
    [1, 0.3963377774, 0.2158037573],
    [1, -0.1055613458, -0.0638541728],
    [1, -0.0894841775, -1.291485548],
];

// The sRGB transfer function, both ways, and the cube root OKLab states its axes through.
function oklch_linear_from_srgb(value) {
    return value <= 0.04045 ? value / 12.92 : Math.pow((value + 0.055) / 1.055, 2.4);
}

function oklch_srgb_from_linear(value) {
    return value <= 0.0031308 ? 12.92 * value : 1.055 * Math.pow(value, 1 / 2.4) - 0.055;
}

function oklch_cube_root(value) {
    return Math.sign(value) * Math.pow(Math.abs(value), 1 / 3);
}

// A matrix applied to a vector, for the conversions stated here.
function oklch_apply(matrix, values) {
    return matrix.map((row) => row[0] * values[0] + row[1] * values[1] + row[2] * values[2]);
}

// The linear sRGB channels a lightness, chroma and hue state.
function oklch_linear_srgb(l, c, h) {
    const radians = (h * Math.PI) / 180;
    const axes = oklch_apply(oklch_lms_from_lab, [l, c * Math.cos(radians), c * Math.sin(radians)]);
    return oklch_apply(oklch_linear_from_lms, axes.map((axis) => axis ** 3));
}

// The lightness, chroma and hue those linear sRGB channels state.
function oklch_from_linear_srgb(red, green, blue) {
    const axes = oklch_apply(oklch_lab_from_lms, oklch_apply(oklch_lms_from_linear, [red, green, blue]).map(oklch_cube_root));
    let hue = (Math.atan2(axes[2], axes[1]) * 180) / Math.PI;
    if (hue < 0) hue += 360;
    return { l: axes[0], c: Math.sqrt(axes[1] * axes[1] + axes[2] * axes[2]), h: hue };
}

// Whether sRGB can show the colour a lightness, chroma and hue state.  The tolerance is a
// rounding of the matrices: a colour at the very corner of the gamut lands a hair outside it.
function oklch_in_gamut(l, c, h) {
    return oklch_linear_srgb(l, c, h).every((channel) => channel >= -0.001 && channel <= 1.001);
}

// The largest chroma sRGB can show at a lightness and hue: halves the chroma until the edge of
// what the screen shows is reached.
function oklch_max_chroma(l, h) {
    if (!oklch_in_gamut(l, 0, h)) return 0;
    let inside = 0;
    let outside = 0.4;
    for (let step = 0; step < 20; step += 1) {
        const middle = (inside + outside) / 2;
        if (oklch_in_gamut(l, middle, h)) inside = middle;
        else outside = middle;
    }
    return inside;
}

// The hex a lightness, chroma and hue is written as, without the `#` a theme value omits.
function hex_from_oklch(l, c, h) {
    const lightness = Math.min(Math.max(l, 0), 1);
    const chroma = Math.min(Math.max(c, 0), oklch_max_chroma(lightness, h));
    return oklch_linear_srgb(lightness, chroma, h)
        .map(oklch_srgb_from_linear)
        .map((channel) => Math.round(Math.min(Math.max(channel, 0), 1) * 255))
        .map((channel) => channel.toString(16).padStart(2, "0"))
        .join("");
}

// The lightness, chroma and hue a hex states, with or without the `#` the theme file omits.
function oklch_from_hex(hex) {
    const digits = hex.replace(/^#/, "");
    const expanded = digits.length === 3 ? digits.replace(/./g, "$&$&") : digits;
    const [red, green, blue] = [0, 2, 4]
        .map((index) => parseInt(expanded.slice(index, index + 2), 16) / 255)
        .map(oklch_linear_from_srgb);
    return oklch_from_linear_srgb(red, green, blue);
}

// The plane a card draws, named by the channel the card states: the card for the lightness draws
// the lightness against the chroma with the hue held, the card for the chroma draws the hue
// against the chroma with the lightness held, and the card for the hue draws the hue against the
// lightness with the chroma held.  The axes are the ones the picker at oklch.com uses, so a card
// and its shape read the same way here as there.  An axis takes the whole of what it states, the
// chroma as wide as OKLCH states it rather than as wide as one lightness allows, so the three
// shapes can be read against each other.
const oklch_slice_planes = [
    { id: "l", horizontal: "l", vertical: "c" },
    { id: "c", horizontal: "h", vertical: "c" },
    { id: "h", horizontal: "h", vertical: "l" },
];

const oklch_axis_extent = { l: 1, c: 0.4, h: 360 };

// The channels a drawing states along its axes, as the shader indexes them.  A card states both
// channels of its plane, and the strip under it states the one channel it is drawn along: both take
// the whole of what the axis states, so a strip and the card above it are the same scale and a
// handle stays under the same colour however the other two channels move.
const oklch_channels = { l: 0, c: 1, h: 2 };

function oklch_card_axes(channel) {
    const plane = oklch_slice_planes.find((entry) => entry.id === channel);
    return {
        axes: [oklch_channels[plane.horizontal], oklch_channels[plane.vertical]],
        extents: [oklch_axis_extent[plane.horizontal], oklch_axis_extent[plane.vertical]],
    };
}

function oklch_strip_axes(channel) {
    return {
        axes: [oklch_channels[channel], -1],
        extents: [oklch_axis_extent[channel], 1],
    };
}

// A matrix as the shader states it: GLSL takes the columns, where the matrices above are stated by
// the row each axis' coefficients sit in.
function oklch_glsl_matrix(rows) {
    const columns = [0, 1, 2].map((column) => rows.map((row) => row[column]).join(", "));
    return "mat3(" + columns.join(", ") + ")";
}

// The shader every drawing is made with: a fragment states the colour of the point its axes state,
// or the colour the screen cannot show falls back to, or the edge between them.  An axis that states
// no channel leaves the held value alone, which is how a strip is drawn: one axis varies and the
// other states nothing, so the strip is the line of colours through the plane.
//
// Every number is a named constant below.  The colour spaces' numbers are what the spaces are; the
// drawing's numbers are the dials, and they are the block marked as such.
function oklch_chart_fragment_shader() {
    return `#version 300 es
precision highp float;

uniform vec2 u_resolution;
uniform ivec2 u_axes;
uniform vec3 u_held;
uniform vec2 u_extent;

out vec4 outColour;

// The colour spaces' own numbers, which are definitions rather than dials: the OKLab matrices, and
// the sRGB transfer function's knee, its linear segment's slope, and its power segment's scale,
// offset and exponent.
const mat3 OKLAB_LMS_FROM_LAB = ${oklch_glsl_matrix(oklch_lms_from_lab)};
const mat3 OKLAB_LINEAR_FROM_LMS = ${oklch_glsl_matrix(oklch_linear_from_lms)};
const float SRGB_KNEE = 0.0031308;
const float SRGB_LINEAR_SLOPE = 12.92;
const float SRGB_POWER_SCALE = 1.055;
const float SRGB_POWER_OFFSET = 0.055;
const float SRGB_POWER_EXPONENT = 1.0 / 2.4;

// The dials.
//
// The three of these that count pixels are integers, because they are thresholds over a pixel grid:
// a rule is a whole number of pixels wide, repeats after a whole number of them, and the boundary
// reaches a whole number of them.  A fraction of a pixel is a fraction of a threshold no display can
// state.
//
// GAMUT_TOLERANCE: how far outside the gamut's cube a colour may land and still count as showable,
// which is the rounding allowance of the matrices.
// FALLBACK_STEPS: how finely the chroma is halved to find the edge of the gamut for a fallback.
// CONTRAST_STEPS: how finely the lightness is bisected to state the mark's contrast.
// EDGE_LINE: how many pixels wide each of the two lines the boundary between the gamut and its
//   fallback is drawn with, one on each side of the curve.
// RULE_CONTRAST: the contrast between the mark's two lines, as a ratio of relative luminance — the
//   quantity the eye reads and the one the mark states everywhere it is drawn.  A ratio, not a
//   lightness difference: the same lightness difference states a strong contrast in the middle of the
//   range and none at all at its dark end.
// RULE_LINE_WIDTH: how many pixels wide each of the rule's two lines is — the pixels one line covers in
//   a row of the display, which is the width a stripe running at a diagonal of the grid has there.  A
//   line one pixel wide is connected only at its corners, so its pixels are not a width; two or more
//   are four-connected and so one width in every row the line runs through.
// RULE_PITCH: after how many pixels the rules repeat.
const float GAMUT_MIN = 0.0;
const float GAMUT_MAX = 1.0;
const float GAMUT_TOLERANCE = 0.001;
const int FALLBACK_STEPS = 10;
const int CONTRAST_STEPS = 12;
// The offset a contrast ratio is stated over, which keeps the ratio finite at black.
const float CONTRAST_OFFSET = 0.05;
const int EDGE_LINE = 2;
const float RULE_CONTRAST = 1.5;
const int RULE_LINE_WIDTH = 2;
const int RULE_WIDTH = RULE_LINE_WIDTH * 2;
const int RULE_PITCH = 32;

// The midpoint a bisection halves at.
const float BISECTION_MIDPOINT = 0.5;

vec3 oklch_to_linear(vec3 lch) {
    float hue = radians(lch.z);
    vec3 lab = vec3(lch.x, lch.y * cos(hue), lch.y * sin(hue));
    vec3 axes = OKLAB_LMS_FROM_LAB * lab;
    return OKLAB_LINEAR_FROM_LMS * (axes * axes * axes);
}

vec3 linear_to_srgb(vec3 value) {
    vec3 low = SRGB_LINEAR_SLOPE * value;
    vec3 high = SRGB_POWER_SCALE * pow(max(value, vec3(GAMUT_MIN)), vec3(SRGB_POWER_EXPONENT))
        - SRGB_POWER_OFFSET;
    return mix(low, high, step(vec3(SRGB_KNEE), value));
}

bool in_gamut(vec3 linear) {
    return all(greaterThanEqual(linear, vec3(GAMUT_MIN - GAMUT_TOLERANCE)))
        && all(lessThanEqual(linear, vec3(GAMUT_MAX + GAMUT_TOLERANCE)));
}

// The channel values a pixel of the canvas states.  A pixel is a whole number of the grid, so a
// sample is stated at its centre: the thresholds of the mark are counted in pixels and a position
// between two of them is a position no display can state.
vec3 lch_at(ivec2 pixel) {
    vec2 fractions = (vec2(pixel) + vec2(BISECTION_MIDPOINT)) / u_resolution;
    vec3 lch = u_held;
    if (u_axes.x >= 0) lch[u_axes.x] = fractions.x * u_extent.x;
    if (u_axes.y >= 0) lch[u_axes.y] = fractions.y * u_extent.y;
    return lch;
}

// The lightness, chroma and hue the screen shows for a colour: the chroma is lowered until the
// screen can show it, which is the colour the picker writes when the point itself is out of range.
vec3 shown_lch(vec3 lch) {
    float inside = GAMUT_MIN;
    float outside = max(lch.y, GAMUT_MIN);
    for (int step = 0; step < FALLBACK_STEPS; step++) {
        float chroma = mix(inside, outside, BISECTION_MIDPOINT);
        if (in_gamut(oklch_to_linear(vec3(lch.x, chroma, lch.z)))) inside = chroma;
        else outside = chroma;
    }
    return vec3(lch.x, inside, lch.z);
}

vec3 colour_of(vec3 lch) {
    return linear_to_srgb(oklch_to_linear(lch));
}

// The relative luminance of a linear sRGB colour, which a contrast ratio is stated over.
float luminance_of(vec3 linear) {
    return dot(linear, vec3(0.2126, 0.7152, 0.0722));
}

// The colour the screen shows at one lightness of the fallback's own hue and chroma, as linear channels.
vec3 shown_linear(float lightness, float chroma, float hue) {
    return oklch_to_linear(shown_lch(vec3(lightness, chroma, hue)));
}

// The contrast two lightnesses of one hue and chroma state: the ratio of their relative luminances, as
// the eye states it.  A ratio rather than a difference, because a difference that reads at mid grey
// states nothing on the same colours near black.
float contrast_of(float one, float other, float chroma, float hue) {
    float a = max(luminance_of(shown_linear(one, chroma, hue)), GAMUT_MIN);
    float b = max(luminance_of(shown_linear(other, chroma, hue)), GAMUT_MIN);
    return (max(a, b) + CONTRAST_OFFSET) / (min(a, b) + CONTRAST_OFFSET);
}

// The lightness on one side of a colour that states a contrast against it, and the contrast it states
// there, which is less than the target where the lightness scale has no room to reach it.
float side_lightness(vec3 shown, bool above, float target, out float reached) {
    float edge = above ? GAMUT_MAX : GAMUT_MIN;
    reached = contrast_of(edge, shown.x, shown.y, shown.z);
    if (reached < target) return edge;
    float low = above ? shown.x : GAMUT_MIN;
    float high = above ? GAMUT_MAX : shown.x;
    for (int step = 0; step < CONTRAST_STEPS; step++) {
        float middle = mix(low, high, BISECTION_MIDPOINT);
        if (contrast_of(middle, shown.x, shown.y, shown.z) >= target) {
            if (above) high = middle;
            else low = middle;
        } else if (above) {
            low = middle;
        } else {
            high = middle;
        }
    }
    reached = target;
    return above ? high : low;
}

// The lightnesses of the mark's two lines, which state the dial's contrast between them.  The pair
// straddles the colour — each line the same contrast from it, which is the mark's free parameter set
// to zero — so long as both sides of the scale reach half of it.  Where a side cannot, it takes what
// the scale gives and the far side makes up the rest, so the contrast between the lines is the dial's
// wherever the mark is drawn.  The single flag states which line is the whole mark where the scale gave one
// side nothing that shows: a rule that kept the one line in half of its width and left the other half
// the colour under it would read as a dash, one dot per pitch, rather than as a rule.
void mark_lines(vec3 shown, out float lighter, out float darker, out int single) {
    float side = sqrt(RULE_CONTRAST);
    float above_reached;
    float below_reached;
    lighter = side_lightness(shown, true, side, above_reached);
    darker = side_lightness(shown, false, side, below_reached);
    single = 0;
    if (above_reached < side) {
        darker = side_lightness(shown, false, RULE_CONTRAST / above_reached, below_reached);
        single = 2;
    } else if (below_reached < side) {
        lighter = side_lightness(shown, true, RULE_CONTRAST / below_reached, above_reached);
        single = 1;
    }
}

// One line of the mark, as the rule states it: the pair's own line where the mark has two, and the one
// line the whole rule states where it has one.
vec3 line_of(vec3 shown, int nth) {
    float lighter;
    float darker;
    int single;
    mark_lines(shown, lighter, darker, single);
    if (single != 0) nth = single;
    return colour_of(shown_lch(vec3(nth == 1 ? lighter : darker, shown.y, shown.z)));
}

// One line of the pair the boundary states across a curve, which keeps both of its lines even where
// the rule is a single line: the two sides of a curve are two colours, and the pair is what states the
// contrast between them.
vec3 pair_line_of(vec3 shown, int nth) {
    float lighter;
    float darker;
    int single;
    mark_lines(shown, lighter, darker, single);
    return colour_of(shown_lch(vec3(nth == 1 ? lighter : darker, shown.y, shown.z)));
}

// Whether a colour of the wanted side of the gamut's edge stands within the boundary's reach of a
// point, which is what puts the point on the boundary.
// Whether a colour of the wanted side of the gamut's edge stands within the boundary's width of a
// pixel, which is what puts the pixel on the boundary.  Both the width and the pixels it counts over
// are whole numbers.
bool edge_reaches(ivec2 pixel, bool want_inside) {
    for (int distance = 1; distance <= EDGE_LINE; distance++) {
        if (in_gamut(oklch_to_linear(lch_at(pixel + ivec2(distance, 0)))) == want_inside
            || in_gamut(oklch_to_linear(lch_at(pixel - ivec2(distance, 0)))) == want_inside
            || in_gamut(oklch_to_linear(lch_at(pixel + ivec2(0, distance)))) == want_inside
            || in_gamut(oklch_to_linear(lch_at(pixel - ivec2(0, distance)))) == want_inside) {
            return true;
        }
    }
    return false;
}

// A point the screen cannot show is drawn as the colour it falls back to, and the part of a slice
// that is a fallback is marked.  The mark says "not a colour you can have", which cannot be said with
// a colour of its own: a colour is the premise of a slice, and a mark drawn in one competes with the
// subject.  It is a rule of two lines that state a contrast between them, placed so that the pair is
// the same contrast from the colour it crosses and straddles it — no direction to flip, so no seam.
//
// The boundary between the gamut and its fallback is the same mark turned along a curve: one line
// inside the gamut and one outside it, so the contrast the mark states is stated *across* the curve.
// Both lines on one side of it would leave the curve itself unstated, which is what the drawing did
// while the two lines were two rings on the fallback's side and the boundary was the faintest thing on
// the card.
void main() {
    // Everything this states is a whole pixel's: which pixel a point is, how far the boundary
    // reaches, and where in the rule's pitch the pixel falls.
    ivec2 pixel = ivec2(gl_FragCoord.xy);
    vec3 lch = lch_at(pixel);
    vec3 linear = oklch_to_linear(lch);
    if (in_gamut(linear)) {
        vec3 plain = colour_of(lch);
        outColour = vec4(edge_reaches(pixel, false) ? pair_line_of(lch, 1) : plain, GAMUT_MAX);
        return;
    }
    vec3 shown = shown_lch(lch);
    if (edge_reaches(pixel, true)) {
        outColour = vec4(pair_line_of(shown, 2), GAMUT_MAX);
        return;
    }
    // The rule runs at a diagonal, counted from the pixel's own position: the diagonal is what states
    // that a colour is a fallback, and it is the same diagonal in a card and in a strip.
    int phase = (pixel.x + pixel.y) % RULE_PITCH;
    if (phase < RULE_WIDTH) {
        bool first = phase < RULE_LINE_WIDTH;
        outColour = vec4(line_of(shown, first ? 1 : 2), GAMUT_MAX);
        return;
    }
    outColour = vec4(colour_of(shown), GAMUT_MAX);
}
`;
}

// The shader that covers the card: three vertices that make a triangle over the whole of it, stated
// by their own index.  Its numbers are the construction of that triangle and nothing else.
function oklch_chart_vertex_shader() {
    return `#version 300 es
void main() {
    vec2 position = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(position * 2.0 - 1.0, 0.0, 1.0);
}
`;
}

const oklch_chart_renderers = new WeakMap();
const oklch_chart_sizes = new WeakMap();
const oklch_chart_rects = new WeakMap();

// The size a drawing is made at, in the canvas' own pixels, and the place it is shown at in the
// page's.  Measuring forces the page to lay out, and a drag states a style every frame, so the page
// measures when the picker opens and when the panes move rather than the drawing or the pointer
// asking every time.
function oklch_chart_measure(canvas) {
    const box = canvas.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    const size = {
        width: Math.max(1, Math.round(box.width * ratio)),
        height: Math.max(1, Math.round(box.height * ratio)),
    };
    // A drawing that is not shown has no place to state, so what it had stands until a frame after
    // it is shown asks again.
    if (box.width && box.height) {
        // The drawing is laid out at exactly the pixels it is made of: the size stated is the size it
        // was measured at snapped to a whole pixel of the display, and the backing store is that same
        // number, so nothing stands between the pixels the shader states and the pixels shown.  A
        // drawing laid out a fraction of a pixel smaller than it was made is a drawing the browser
        // scales, and a line stated a pixel wide is shown as segments it dropped.
        canvas.style.width = size.width / ratio + "px";
        canvas.style.height = size.height / ratio + "px";
        oklch_chart_sizes.set(canvas, size);
        oklch_chart_rects.set(canvas, {
            left: box.left,
            top: box.top,
            width: size.width / ratio,
            height: size.height / ratio,
        });
    }
    return size;
}

// The context and program one card is drawn with, built the first time the card is drawn.
function oklch_chart_renderer(canvas) {
    const cached = oklch_chart_renderers.get(canvas);
    if (cached) return cached;
    const gl = canvas.getContext("webgl2", { alpha: true, antialias: false, premultipliedAlpha: false });
    if (!gl) return null;
    const compile = (type, source) => {
        const shader = gl.createShader(type);
        gl.shaderSource(shader, source);
        gl.compileShader(shader);
        return shader;
    };
    const program = gl.createProgram();
    gl.attachShader(program, compile(gl.VERTEX_SHADER, oklch_chart_vertex_shader()));
    gl.attachShader(program, compile(gl.FRAGMENT_SHADER, oklch_chart_fragment_shader()));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        console.error("the colour chart shader did not link:", gl.getProgramInfoLog(program));
        return null;
    }
    const renderer = {
        gl,
        program,
        uniforms: {
            resolution: gl.getUniformLocation(program, "u_resolution"),
            axes: gl.getUniformLocation(program, "u_axes"),
            held: gl.getUniformLocation(program, "u_held"),
            extent: gl.getUniformLocation(program, "u_extent"),
        },
    };
    oklch_chart_renderers.set(canvas, renderer);
    return renderer;
}

// A drawing, made in one call at the size it is shown at: the channels its axes state, and how far
// each of them reaches.
function draw_oklch_chart(canvas, axes, extents, picker) {
    // A display ratio that changed since the drawing was measured is a resampling of everything the
    // drawing states — a line stated one pixel wide is then shown a fraction of a pixel wide or two,
    // and rows are dropped and doubled to keep the size.  So the drawing is measured again rather than
    // made for the ratio it was measured at.
    const ratio = window.devicePixelRatio || 1;
    const measured = oklch_chart_sizes.get(canvas);
    const rect = oklch_chart_rects.get(canvas);
    const stale = !measured || !rect
        || Math.round(rect.width * ratio) !== measured.width
        || Math.round(rect.height * ratio) !== measured.height;
    const size = stale ? oklch_chart_measure(canvas) : measured;
    const renderer = oklch_chart_renderer(canvas);
    if (!renderer) return;
    const { gl, program, uniforms } = renderer;
    const { width, height } = size;
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    gl.viewport(0, 0, width, height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(program);
    gl.uniform2f(uniforms.resolution, width, height);
    gl.uniform2i(uniforms.axes, axes[0], axes[1]);
    gl.uniform3f(uniforms.held, picker.l, picker.c, picker.h);
    gl.uniform2f(uniforms.extent, extents[0], extents[1]);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
}
