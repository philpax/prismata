# Prismata

Internal voxel-based research prototype at Ambient (released with consent).

Models for ComfyUI:

- <https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0/resolve/main/sd_xl_base_1.0_0.9vae.safetensors> to `models/checkpoints`
- <https://huggingface.co/xinsir/controlnet-union-sdxl-1.0/resolve/main/diffusion_pytorch_model_promax.safetensors> in `models/controlnet`
- <https://huggingface.co/spacepxl/ml-depth-pro/blob/main/depth_pro.fp16.safetensors> in `models/depth/ml-depth-pro`
- <https://huggingface.co/lllyasviel/fooocus_inpaint/resolve/main/fooocus_inpaint_head.pth> and <https://huggingface.co/lllyasviel/fooocus_inpaint/resolve/main/inpaint_v26.fooocus.patch> in `models/inpaint`

Once you have ComfyUI up and running, install ComfyUI-Manager so that you can install custom nodes: <https://github.com/ltdrdata/ComfyUI-Manager?tab=readme-ov-file#installation>

Using ComfyUI-Manager:

- Install `ComfyUI-Depth-Pro`.
- Install `ComfyUI Inpaint Nodes`.

Install our custom Prismata node:

- `rsync -avz -e "ssh -p $PORT" ComfyUI-Prismata root@$IP:/workspace/ComfyUI/custom_nodes/`
- `ssh` then `cd /workspace/ComfyUI/custom_nodes/ComfyUI-Prismata` then `pip install -r requirements.txt`
