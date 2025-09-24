#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput}
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput}
}
#endif

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = in.color;
    return out;
}
