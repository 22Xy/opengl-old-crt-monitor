#version 410
precision mediump float;

in vec2 vert;

out vec4 out_color;

uniform sampler2D in_tex;
uniform float time;

void main() {
    vec2 adjusted_coord = vert.yx;
    adjusted_coord.y = 1.0 - adjusted_coord.y;
    adjusted_coord *= 2;
    adjusted_coord -= 0.5;

    vec4 background = vec4(
        0.0,
        (sin(adjusted_coord.y * 200.0 + time) + 1.0) / 10.0,
        0.0,
        1.0);

    if (adjusted_coord.x > 1.0 || adjusted_coord.y > 1.0 || adjusted_coord.x < 0.0 || adjusted_coord.y < 0) {
        out_color = background;
    } else {
        vec4 sampled_color = texture(in_tex, adjusted_coord);
        float threshold = 0.1; // Adjust this tolerance as needed

        // Check if the color is close to the key color (e.g., black)
        if (sampled_color.r < threshold && sampled_color.g < threshold && sampled_color.b < threshold) {
            out_color = background;
        } else {
            out_color = vec4(0.0, sampled_color.g, 0.0, 1.0);
        }
    }
}
