import os
import random

os.environ["OPENCV_IO_ENABLE_OPENEXR"] = "1"

import cv2 as cv
import numpy as np

import folder_paths
from comfy.cli_args import args


class PrismataSave:
    def __init__(self):
        self.output_dir = folder_paths.get_temp_directory()
        self.type = "temp"
        self.prefix_append = "_temp_" + "".join(
            random.choice("abcdefghijklmnopqrstupvxyz") for x in range(5)
        )

    @classmethod
    def INPUT_TYPES(s):
        return {
            "required": {
                "images": ("IMAGE",),
                "focal_avg": ("*", {}),
            }
        }

    @classmethod
    def VALIDATE_INPUTS(s, input_types):
        return True

    RETURN_TYPES = ()
    FUNCTION = "save_images"

    OUTPUT_NODE = True

    CATEGORY = "image"

    def save_images(
        self,
        images,
        focal_avg,
    ):
        filename_prefix = "Prismata" + self.prefix_append
        full_output_folder, filename, counter, subfolder, filename_prefix = (
            folder_paths.get_save_image_path(
                filename_prefix, self.output_dir, images[0].shape[1], images[0].shape[0]
            )
        )
        results = list()

        linear = images.cpu().numpy().astype(np.float32)

        bgr = linear.copy()
        bgr[:, :, :, 0] = linear[:, :, :, 2]  # flip RGB to BGR for opencv
        bgr[:, :, :, 2] = linear[:, :, :, 0]
        if bgr.shape[-1] > 3:
            bgr[:, :, :, 3] = np.clip(1 - linear[:, :, :, 3], 0, 1)  # invert alpha

        batch_size = linear.shape[0]
        for batch_number in range(batch_size):
            filename_with_batch_num = filename.replace("%batch_num%", str(batch_number))
            file = f"{filename_with_batch_num}_{counter:05}_.exr"
            writepath = os.path.join(full_output_folder, file)
            cv.imwrite(writepath, bgr[batch_number])
            results.append(
                {"filename": file, "subfolder": subfolder, "type": self.type}
            )
            counter += 1

        return {"ui": {"images": results, "text": [str(focal_avg)]}}


NODE_CLASS_MAPPINGS = {
    "PrismataSave": PrismataSave,
}

NODE_DISPLAY_NAME_MAPPINGS = {
    "PrismataSave": "Prismata Save",
}
