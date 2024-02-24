// dxc -T cs_6_6 -E main alpha_blend.hlsl -Fo alpha_blend.cso
#define THREAD_GROUP_SIZE_X 16
#define THREAD_GROUP_SIZE_Y 16

RWTexture2D<float4> dst: register(u0); // UAV
Texture2D<float4> src: register(t0); // SRV

SamplerState smp: register(s0);

[numthreads(THREAD_GROUP_SIZE_X, THREAD_GROUP_SIZE_Y, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 pixel = dispatchThreadID.xy;

    // float4 srcColor = src.Sample(smp, pixel);
    float4 srcColor = src.Load(int3(pixel, 0));
    float4 dstColor = dst[pixel];

    float4 outColor = srcColor + dstColor * (1 - srcColor.a);

    dst[pixel] = outColor;
}

