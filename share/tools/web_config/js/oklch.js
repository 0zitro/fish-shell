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

// The mark a fallback is drawn with, and the field it is drawn on.
//
// Two reagents diffuse at different rates and one catalyses the other's production; what settles out
// is a labyrinth.  It states no direction and no period, and its isocontours cannot cross, so a pair
// of lines drawn either side of one is a mark that repeats nowhere and meets nothing — which is what
// the mark has to be, since a direction it carried could be shared with the edge it sits beside.
//
// The field wraps at its edges, so it tiles without a seam, and it is sampled at a stated number of
// device pixels per cell, so every drawing states the same sizes as every other.  The generator counts
// in cells: the widths of the mark are cells of this field, converted to pixels by that scale.
//
// The feed and the kill decide what settles out.  Stated for worms, the reaction settles into long
// strands that double back and meet themselves, rather than into a field of round spots with a few
// strands between them: the strands are what the mark is drawn on, and a spot is a shape the mark would
// have to close around.
const oklch_hatch_columns = 128;
const oklch_hatch_rows = 88;
const oklch_hatch_steps = 2400;
const oklch_hatch_feed = 0.05;
const oklch_hatch_kill = 0.0615;
const oklch_hatch_diffusion = [0.16, 0.08];
const oklch_hatch_scale = 3;
// Where in the field's own range the pair of lines sits.
const oklch_hatch_level = 0.75;

let oklch_hatch_field = null;

// The field, simulated once and kept: the pattern is a property of the parameters and not of the
// drawing, so every drawing states the same one at the size it is made at.
function oklch_hatch_texture() {
    if (oklch_hatch_field) return oklch_hatch_field;
    const columns = oklch_hatch_columns;
    const rows = oklch_hatch_rows;
    const cells = columns * rows;
    let first = new Float32Array(cells);
    let second = new Float32Array(cells);
    // The pattern that settles is the parameters' own, so the start is a scatter rather than a shape:
    // a seed with a shape states that shape.
    for (let index = 0; index < cells; index += 1) {
        first[index] = 1;
        second[index] = Math.random() * 0.02;
    }
    for (let index = 0; index < 24; index += 1) {
        const column = Math.floor(Math.random() * columns);
        const row = Math.floor(Math.random() * rows);
        second[row * columns + column] = 1;
    }
    const left = new Int32Array(columns);
    const right = new Int32Array(columns);
    for (let column = 0; column < columns; column += 1) {
        left[column] = (column + columns - 1) % columns;
        right[column] = (column + 1) % columns;
    }
    let nextFirst = new Float32Array(cells);
    let nextSecond = new Float32Array(cells);
    const [diffusionFirst, diffusionSecond] = oklch_hatch_diffusion;
    for (let step = 0; step < oklch_hatch_steps; step += 1) {
        for (let row = 0; row < rows; row += 1) {
            const here = row * columns;
            const up = ((row + rows - 1) % rows) * columns;
            const down = ((row + 1) % rows) * columns;
            for (let column = 0; column < columns; column += 1) {
                const index = here + column;
                const a = first[index];
                const b = second[index];
                const across =
                    first[here + left[column]] + first[here + right[column]]
                    + first[up + column] + first[down + column] - 4 * a;
                const acrossSecond =
                    second[here + left[column]] + second[here + right[column]]
                    + second[up + column] + second[down + column] - 4 * b;
                const reaction = a * b * b;
                nextFirst[index] =
                    a + diffusionFirst * across - reaction + oklch_hatch_feed * (1 - a);
                nextSecond[index] =
                    b + diffusionSecond * acrossSecond + reaction
                    - (oklch_hatch_feed + oklch_hatch_kill) * b;
            }
        }
        const spare = first;
        first = nextFirst;
        nextFirst = spare;
        const spareSecond = second;
        second = nextSecond;
        nextSecond = spareSecond;
    }
    // The field the screen samples states its own range, so the dials above are fractions of it rather
    // than numbers of a simulation that states no scale.
    let low = Infinity;
    let high = -Infinity;
    for (const value of second) {
        if (value < low) low = value;
        if (value > high) high = value;
    }
    const span = high - low > 0 ? high - low : 1;
    const data = new Uint8Array(cells);
    for (let index = 0; index < cells; index += 1) {
        data[index] = Math.round(((second[index] - low) / span) * 255);
    }
    oklch_hatch_field = { data, columns, rows };
    return oklch_hatch_field;
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
uniform sampler2D u_hatch;
uniform vec2 u_hatch_step;

out vec4 outColour;

// The colour spaces' own numbers, which are definitions rather than dials: the OKLab matrices both
// ways, the exponent its axes are the cube root of, and the sRGB transfer function's knee, its linear
// segment's slope, and its power segment's scale, offset and exponent.
const mat3 OKLAB_LMS_FROM_LAB = ${oklch_glsl_matrix(oklch_lms_from_lab)};
const mat3 OKLAB_LINEAR_FROM_LMS = ${oklch_glsl_matrix(oklch_linear_from_lms)};
const mat3 OKLAB_LMS_FROM_LINEAR = ${oklch_glsl_matrix(oklch_lms_from_linear)};
const mat3 OKLAB_LAB_FROM_LMS = ${oklch_glsl_matrix(oklch_lab_from_lms)};
const float OKLAB_ROOT_EXPONENT = 1.0 / 3.0;
const float SRGB_KNEE = 0.0031308;
const float SRGB_LINEAR_SLOPE = 12.92;
const float SRGB_POWER_SCALE = 1.055;
const float SRGB_POWER_OFFSET = 0.055;
const float SRGB_POWER_EXPONENT = 1.0 / 2.4;

// The dials.
//
// The boundary's width is an integer, because it is a threshold over a pixel grid: a fraction of a
// pixel is a fraction of a threshold no display can state.  The mark's own widths are stated in cells
// of the hatch field, which the scale of that field converts to pixels.
//
// GAMUT_TOLERANCE: how far outside the gamut's cube a colour may land and still count as showable,
// which is the rounding allowance of the matrices.
// FALLBACK_STEPS: how finely the chroma is halved to find the edge of the gamut, for the colour a
//   fallback shows and for a line of the mark that stands wider than the screen shows there.
// LUMINANCE_STEPS: how finely the lightness is halved to reach the luminance a line of the mark
//   states.  Luminance is not lightness: at one lightness a chromatic colour states less of it than a
//   grey does, so the lightness a luminance states is a root, which the halving reaches and a scale
//   cannot.
// TINT_ROUNDS: how many times a line's chroma gives way to the chroma the screen shows at the
//   lightness the line stands at.  The width the screen shows at a lightness depends on that
//   lightness, and the lightness depends on the width, so the two are stated by iterating them; a
//   line already inside the gamut leaves the loop on its first round.
// EDGE_LINE: how many pixels wide each of the two lines the boundary between the gamut and its
//   fallback is drawn with, one on each side of the curve.
// RULE_CONTRAST: the contrast between the mark's two lines, as a ratio of relative luminance — the
//   quantity the eye reads and the one the mark states everywhere it is drawn.  A ratio, not a
//   lightness difference: the same lightness difference states a strong contrast in the middle of the
//   range and none at all at its dark end.
// HATCH_LEVEL: where in the hatch field's own range the pair of lines sits, as a fraction of it.
// HATCH_LINE: how many pixels wide each of the rule's two lines is, counted from the isocontour the pair
//   straddles.
const float GAMUT_MIN = 0.0;
const float GAMUT_MAX = 1.0;
const float GAMUT_TOLERANCE = 0.001;
const int FALLBACK_STEPS = 10;
const int LUMINANCE_STEPS = 16;
const int TINT_ROUNDS = 2;
// The offset a contrast ratio is stated over, which keeps the ratio finite at black: a ratio divides
// by a luminance and the darkest colour of the display is one the cube states at zero.
const float CONTRAST_OFFSET = 0.05;
const int EDGE_LINE = 2;
const float RULE_CONTRAST = 1.8;
const int HATCH_LINE = 2;
const float HATCH_LEVEL = ${oklch_hatch_level};

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

// The chroma the screen shows at a lightness and hue: the stated one where the screen shows it, and
// the widest one it shows there where it does not.
float shown_chroma(float lightness, float chroma, float hue) {
    if (in_gamut(oklch_to_linear(vec3(lightness, chroma, hue)))) return chroma;
    float inside = GAMUT_MIN;
    float outside = max(chroma, GAMUT_MIN);
    for (int step = 0; step < FALLBACK_STEPS; step++) {
        float middle = mix(inside, outside, BISECTION_MIDPOINT);
        if (in_gamut(oklch_to_linear(vec3(lightness, middle, hue)))) inside = middle;
        else outside = middle;
    }
    return inside;
}

// The lightness, chroma and hue the screen shows for a colour: the chroma is lowered until the
// screen can show it, which is the colour the picker writes when the point itself is out of range.
vec3 shown_lch(vec3 lch) {
    return vec3(lch.x, shown_chroma(lch.x, lch.y, lch.z), lch.z);
}

vec3 colour_of(vec3 lch) {
    return linear_to_srgb(oklch_to_linear(lch));
}

// The relative luminance of a linear sRGB colour, which a contrast ratio is stated over.
float luminance_of(vec3 linear) {
    return dot(linear, vec3(0.2126, 0.7152, 0.0722));
}

// The mark, and the constraints it states.  The mark's pattern is one thing and these are another: a
// rule of stripes, the isocontours of a field, a field of blobs — any of them states where the mark goes
// and none of them decides what the mark is worth.  Those decisions are these, taken over the whole
// drawing.
//
// Let bg be the colour under a point of the mark, and let the mark at that point state two lines, one
// lighter than bg and one darker, with colours light and dark.  Then:
//
//   contrast(light, dark) = RULE_CONTRAST, everywhere.  The two lines are one pair whatever they are
//   drawn on, so the mark is one mark and not a different one over each colour.
//
//   contrast(light, bg) and contrast(dark, bg) are each as uniform as the drawing allows.  A line's own
//   colour is what changes from point to point of the drawing, and the change is the cost: the mark is
//   uniform where neither line changes colour, and a line whose colour moves is a second statement
//   inside the mark.
//
//   |contrast(light, bg) - contrast(dark, bg)| = RULE_BALANCE, everywhere, for one global dial.  The two
//   lines treat the colour under them alike, so neither of them is the favoured one over any part of the
//   drawing; RULE_BALANCE stated at zero makes the pair straddle bg.
//
// The three cannot all hold at the ends of the luminance scale: a pair straddling a colour at the very
// bottom of it must put one line below the scale or move both.  What is given up there is the balance,
// and only as much of it as the scale forces, because the pair's own contrast is what makes the two lines
// one mark.
//
// RULE_CHROMA states how much of the colour under the mark the two lines carry: each line states the
// drawing's own hue and this fraction of its chroma, so the tint a line contributes is the drawing's
// colour, as much of it as the dial says and no more.  The fraction is of the chroma and not of the
// channel values, because a chroma is what the eye reads as colourfulness: the channel values of a dark
// colour stand close together however saturated that colour is, and a fraction of them states a line
// whose saturation is the widest the screen shows at the line's luminance rather than the drawing's.  A
// line holds its luminance whatever this is stated at, so the pair's contrast and the pair's balance are
// the dials' whichever way it is stated.  At zero the pair is two greys and its colour is the same
// everywhere; at one each line is the colour under it at that line's own lightness.
const float RULE_BALANCE = 0.0;
const float RULE_CHROMA = 0.5;

// The luminance a contrast from a luminance states.  The ratio is stated over the luminance offset by
// CONTRAST_OFFSET, so this is a closed form: the luminance a line stands at is stated from the colour
// under it, and what is solved for is the colour at that luminance.
float luminance_at_contrast(float luminance, float ratio) {
    return ratio * (luminance + CONTRAST_OFFSET) - CONTRAST_OFFSET;
}

// The offset a hue and a chroma state on the cone response's axes.  The lightness states its own
// coefficient in each of those axes, and that coefficient is one, so the axes at a lightness are that
// lightness plus this offset.
vec3 oklab_offset(float chroma, float hue) {
    float turned = radians(hue);
    return OKLAB_LMS_FROM_LAB * vec3(0.0, chroma * cos(turned), chroma * sin(turned));
}

// The linear sRGB colour the cone response's axes state.
vec3 linear_at(vec3 offset, float lightness) {
    vec3 axes = offset + vec3(lightness);
    return OKLAB_LINEAR_FROM_LMS * (axes * axes * axes);
}

// The hue and the chroma a linear sRGB colour states: the chromatic axes of its cone response, and the
// distance and the angle that pair states.
void hue_chroma_of(vec3 linear, out float hue, out float chroma) {
    vec3 axes = OKLAB_LMS_FROM_LINEAR * linear;
    axes = OKLAB_LAB_FROM_LMS * (sign(axes) * pow(abs(axes), vec3(OKLAB_ROOT_EXPONENT)));
    chroma = length(axes.yz);
    hue = degrees(atan(axes.z, axes.y));
    if (hue < 0.0) hue += 360.0;
}

// The lightness whose luminance is the one asked for, at a stated hue and chroma.  Luminance rises with
// lightness at a stated hue and chroma, and the axes it is stated over are the lightness plus the hue's
// and chroma's offset, so halving the lightness reaches the one lightness that states it.
float lightness_at(vec3 offset, float target) {
    float below = GAMUT_MIN;
    float above = GAMUT_MAX;
    for (int step = 0; step < LUMINANCE_STEPS; step++) {
        float lightness = mix(below, above, BISECTION_MIDPOINT);
        if (luminance_of(linear_at(offset, lightness)) < target) below = lightness;
        else above = lightness;
    }
    return below;
}

// The colour a line of the mark states: the colour under the mark with its lightness moved to the
// luminance the contrast asks for, keeping its hue and the stated fraction of its chroma.
//
// The chroma is stated at the line's own lightness, and the ends of the scale show less chroma than
// its middle does, so a line there gives up the width and keeps the hue.  Keeping the hue is what keeps
// the line the drawing's colour: a channel value below the gamut states a hue no colour of the drawing
// has, and over a nearly black drawing that is a saturated green or orange where the drawing states a
// hue too dark to read at all.
vec3 linear_at_luminance(vec3 linear, float target) {
    float hue;
    float chroma;
    hue_chroma_of(linear, hue, chroma);
    chroma *= RULE_CHROMA;
    vec3 offset = oklab_offset(chroma, hue);
    float lightness = lightness_at(offset, target);
    for (int round = 0; round < TINT_ROUNDS; round++) {
        float shown = shown_chroma(lightness, chroma, hue);
        if (shown >= chroma) break;
        chroma = shown;
        offset = oklab_offset(chroma, hue);
        lightness = lightness_at(offset, target);
    }
    return linear_at(offset, lightness);
}

// The two lines of the mark.  Each is stated at a contrast from the colour under them, a square root of
// the dial either way, which straddles that colour with the balance at zero.  Where a line would stand
// outside the scale the pair is shifted the least distance that keeps both inside it and the other line
// follows the shift: the pair's own contrast is what makes the two lines one mark, so it is the dial's
// wherever the mark is drawn.
void mark_linears(vec3 linear, out vec3 lighter, out vec3 darker) {
    float luminance = clamp(luminance_of(linear), GAMUT_MIN, GAMUT_MAX);
    float side = sqrt(RULE_CONTRAST);
    float balance = RULE_BALANCE * BISECTION_MIDPOINT;
    float lift = side * (GAMUT_MAX + balance);
    float drop = GAMUT_MAX / lift;
    float ratio = lift / drop;
    float high = luminance_at_contrast(luminance, lift);
    float low = luminance_at_contrast(luminance, drop);
    if (low < GAMUT_MIN) {
        low = GAMUT_MIN;
        high = luminance_at_contrast(low, ratio);
    }
    if (high > GAMUT_MAX) {
        high = GAMUT_MAX;
        low = luminance_at_contrast(high, GAMUT_MAX / ratio);
    }
    lighter = linear_at_luminance(linear, high);
    darker = linear_at_luminance(linear, low);
}

// One line of the mark, in the colour the screen writes.
vec3 line_of(vec3 linear, int nth) {
    vec3 lighter;
    vec3 darker;
    mark_linears(linear, lighter, darker);
    return linear_to_srgb(nth == 1 ? lighter : darker);
}

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

// The colour the gamut states at the edge nearest a pixel, which is the point the two materials of a
// boundary are compared at: the colour inside is a colour of the drawing, and the colour it falls back
// to is stated by the same lightness and hue with the chroma the screen can show.
vec3 inside_lch(ivec2 pixel, bool inside) {
    if (inside) return lch_at(pixel);
    for (int distance = 1; distance <= EDGE_LINE; distance++) {
        for (int direction = 0; direction < 4; direction++) {
            ivec2 offset = direction == 0 ? ivec2(distance, 0)
                : direction == 1 ? ivec2(-distance, 0)
                : direction == 2 ? ivec2(0, distance)
                : ivec2(0, -distance);
            vec3 other = lch_at(pixel + offset);
            if (in_gamut(oklch_to_linear(other))) return other;
        }
    }
    return lch_at(pixel);
}

// Which line of the pair the gamut's side of an edge states, and so which line the side across from it
// states.
//
// The two materials of a boundary are one colour: a fallback is the colour it stands for with the
// chroma the screen can show, and at the edge that chroma is the colour's own.  So the pair is the pair
// of the colour inside, and both sides of the edge must take their lines from that one colour — one
// line each — or the two sides state a pair of two different pairs and the contrast across the edge is
// neither line's.
//
// Which side takes which line is then the relief the edge is drawn with: the brighter of the two sides
// takes the darker line, so the light line lies on the darker material.  It is stated at the colour
// inside the edge, the side the two materials are both stated on.
int inside_role(vec3 inside) {
    float insideLuminance = luminance_of(oklch_to_linear(inside));
    float fallbackLuminance = luminance_of(oklch_to_linear(shown_lch(inside)));
    return insideLuminance > fallbackLuminance ? 2 : 1;
}

// A point the screen cannot show is drawn as the colour it falls back to, and the part of a slice
// that is a fallback is marked.  The mark says "not a colour you can have", which cannot be said with
// a colour of its own: a colour is the premise of a slice, and a mark drawn in one competes with the
// subject.  It is a rule of two lines that state a contrast between them, placed so that the pair is
// the same contrast from the colour it crosses and straddles it — no direction to flip, so no seam.
//
// The boundary between the gamut and its fallback is the same mark turned along a curve: the pair
// straddles the curve, so the contrast the mark states is stated *across* it.  Which of the two lines
// is drawn on which side is decided at the gamut's side of the edge by the two materials the curve
// separates, so the roles follow the colours the edge sits between rather than the side of the edge.
void main() {
    // Everything this states is a whole pixel's: which pixel a point is, how far the boundary
    // reaches, and where in the hatch field the pixel falls.
    ivec2 pixel = ivec2(gl_FragCoord.xy);
    vec3 lch = lch_at(pixel);
    vec3 linear = oklch_to_linear(lch);
    if (in_gamut(linear)) {
        if (edge_reaches(pixel, false)) {
            outColour = vec4(line_of(linear, inside_role(lch)), GAMUT_MAX);
        } else {
            outColour = vec4(colour_of(lch), GAMUT_MAX);
        }
        return;
    }
    vec3 shown = shown_lch(lch);
    if (edge_reaches(pixel, true)) {
        // The pair across the edge is the pair of the colour inside it, and this side states the line the
        // inside does not.
        vec3 inside = inside_lch(pixel, false);
        int role = inside_role(inside);
        outColour = vec4(line_of(oklch_to_linear(inside), role == 1 ? 2 : 1), GAMUT_MAX);
        return;
    }
    // The rule is the hatch field's isocontours: the pair is the pixels within a stated distance of one,
    // the pixels on the near side of it stating one line and the pixels on the far side the other, with
    // nothing between them, because that adjacency is where the pair's contrast is read.  Which pixels
    // those are is decided by where the field crosses the level, counted in pixels, not by how far the
    // field's value stands from it: a value span is a width the field's gradient decides, and where the
    // field is flat that width is a whole region rather than two lines.
    vec2 place = vec2(pixel) * u_hatch_step;
    float level = texture(u_hatch, place).r - HATCH_LEVEL;
    bool crossed = false;
    for (int step = 1; step <= HATCH_LINE; step++) {
        float offset = float(step);
        float alongX = texture(u_hatch, place + vec2(offset * u_hatch_step.x, 0.0)).r - HATCH_LEVEL;
        float backX = texture(u_hatch, place - vec2(offset * u_hatch_step.x, 0.0)).r - HATCH_LEVEL;
        float alongY = texture(u_hatch, place + vec2(0.0, offset * u_hatch_step.y)).r - HATCH_LEVEL;
        float backY = texture(u_hatch, place - vec2(0.0, offset * u_hatch_step.y)).r - HATCH_LEVEL;
        crossed = crossed || level * alongX <= GAMUT_MIN || level * backX <= GAMUT_MIN
            || level * alongY <= GAMUT_MIN || level * backY <= GAMUT_MIN;
    }
    if (crossed) {
        outColour = vec4(line_of(oklch_to_linear(shown), level < GAMUT_MIN ? 1 : 2), GAMUT_MAX);
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
    // The hatch field belongs to the context it is sampled in, so each drawing's context states it:
    // the field itself is simulated once and stated here as it is.
    const field = oklch_hatch_texture();
    const hatch = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, hatch);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texImage2D(
        gl.TEXTURE_2D, 0, gl.R8, field.columns, field.rows, 0, gl.RED, gl.UNSIGNED_BYTE, field.data,
    );
    const renderer = {
        gl,
        program,
        hatch,
        uniforms: {
            resolution: gl.getUniformLocation(program, "u_resolution"),
            axes: gl.getUniformLocation(program, "u_axes"),
            held: gl.getUniformLocation(program, "u_held"),
            extent: gl.getUniformLocation(program, "u_extent"),
            hatch: gl.getUniformLocation(program, "u_hatch"),
            hatchStep: gl.getUniformLocation(program, "u_hatch_step"),
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
    const { gl, program, uniforms, hatch } = renderer;
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
    // A pixel states the field at a stated number of pixels per cell, so the mark is the same size in
    // every drawing whatever the drawing's own size.
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, hatch);
    gl.uniform1i(uniforms.hatch, 0);
    const field = oklch_hatch_texture();
    gl.uniform2f(
        uniforms.hatchStep,
        1 / (oklch_hatch_scale * field.columns),
        1 / (oklch_hatch_scale * field.rows),
    );
    gl.drawArrays(gl.TRIANGLES, 0, 3);
}
