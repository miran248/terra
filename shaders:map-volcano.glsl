// based on https://thebookofshaders.com/edit.php?log=161127230743

#ifdef GL_ES
precision mediump float;
#endif

uniform vec2 u_resolution;
uniform vec2 u_mouse;
uniform float u_time;

vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec2 mod289(vec2 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec3 permute(vec3 x) { return mod289(((x*34.0)+1.0)*x); }

float snoise(vec2 v) {
    const vec4 C = vec4(0.211324865405187,  // (3.0-sqrt(3.0))/6.0
                        0.366025403784439,  // 0.5*(sqrt(3.0)-1.0)
                        -0.577350269189626,  // -1.0 + 2.0 * C.x
                        0.024390243902439); // 1.0 / 41.0
    vec2 i  = floor(v + dot(v, C.yy) );
    vec2 x0 = v -   i + dot(i, C.xx);
    vec2 i1;
    i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
    vec4 x12 = x0.xyxy + C.xxzz;
    x12.xy -= i1;
    i = mod289(i); // Avoid truncation effects in permutation
    vec3 p = permute( permute( i.y + vec3(0.0, i1.y, 1.0 ))
        + i.x + vec3(0.0, i1.x, 1.0 ));

    vec3 m = max(0.5 - vec3(dot(x0,x0), dot(x12.xy,x12.xy), dot(x12.zw,x12.zw)), 0.0);
    m = m*m ;
    m = m*m ;
    vec3 x = 2.0 * fract(p * C.www) - 1.0;
    vec3 h = abs(x) - 0.5;
    vec3 ox = floor(x + 0.5);
    vec3 a0 = x - ox;
    m *= 1.79284291400159 - 0.85373472095314 * ( a0*a0 + h*h );
    vec3 g;
    g.x  = a0.x  * x0.x  + h.x  * x0.y;
    g.yz = a0.yz * x12.xz + h.yz * x12.yw;
    return 130.0 * dot(m, g);
}

float easeIn(float x) {
    return 1. - cos((x * 3.14) / 2.);
}
float easeOut(float x) {
    return sin((x * 3.14) / 2.);
}
float easeInOut(float x) {
    return -(cos(3.14 * x) - 1.) / 2.;
}

float steps(float count, float x) {
    return floor(x * count) / count;
}

float layer(vec2 offset, float zoom, float ratio) {
    vec2 st = gl_FragCoord.xy / u_resolution.xy + offset;
    st.x *= u_resolution.x / u_resolution.y;
    return easeIn(snoise(st / zoom)) * ratio + (1. - ratio);
}

float island(float distance) {
    float island = 1. - easeIn(distance);
    float layers = 0.5;
    layers += easeIn(layer(vec2(0), .4, .9)) * .4;
    layers += easeIn(layer(vec2(1), .3, .9)) * .3;
    layers += easeIn(layer(vec2(2), .2, .9)) * .2;
    layers += easeIn(layer(vec2(3), .1, .9)) * .1;
    layers *= island;
    return clamp(layers, 0., 1.);
}

float peak(float distance) {
    float island = 1. - easeIn(clamp(distance * 2., 0., 1.));
    float layers = 0.;
    layers += easeIn(layer(vec2(0), .1, .1)) * .1;
    layers += easeIn(layer(vec2(1), .2, .1)) * .2;
    layers += easeIn(layer(vec2(2), .3, .1)) * .3;
    layers += easeIn(layer(vec2(3), .4, .1)) * .4;
    layers *= island;
    return clamp(layers * 5. - 4.25, 0., 1.);
}

void main() {
    vec2 center = vec2(.5, .5);
    float distance = clamp(distance(center, gl_FragCoord.xy / u_resolution.xy) * 2., 0., 1.);
    float layers = 0.;
    layers += island(distance);
    layers += peak(distance);
    layers = clamp(layers, 0., 1.);
    layers = steps(8., layers);
    gl_FragColor = vec4(vec3(layers), 1.0);
}
