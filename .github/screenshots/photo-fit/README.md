# Photo fitting: before and after

These comparisons use the **same real photograph** in the actual before and after application builds. Left is **before** (`50b6bd0`); right is **after** (`f6e015a`). Both builds use the same local Chromium, fixed clock, random seed, photo metadata, full-screen state and **800×1100 portrait viewport**. The photograph is not retouched. This viewport makes cropping and fixed-frame overflow particularly easy to see.

## Single full-screen photo

![Before: cropped to fill the screen. After: the complete photo fits with letterboxing.](photo-fitting-before-after.png)

## Polaroid in full screen

![Before: the fixed frame extends beyond the screen edges. After: the full frame and image remain visible.](polaroid-before-after.png)

## Photograph and provenance

`source-rocket.jpg` is SpaceX's DSCOVR/Falcon 9 launch photograph, released into the public domain. It is the unmodified sample from [scikit-image v0.18.3](https://github.com/scikit-image/scikit-image/blob/v0.18.3/skimage/data/rocket.jpg). Its [dataset documentation](https://github.com/scikit-image/scikit-image/blob/v0.18.3/skimage/data/__init__.py) identifies [SpaceX's original photograph](https://www.flickr.com/photos/spacexphotos/16511594820/in/photostream/) and its public-domain status. These are sample images, not household photos.

## Validation status

These are review illustrations, **not Playwright CI baselines**. Synthetic edge-marked fixtures remain in the automated geometry tests, where they make clipping detectable.

The CI baselines were regenerated in `mcr.microsoft.com/playwright:v1.65.0-noble` and committed in `0779f43`. After merging the latest `main`, `3c6e64d` refreshed the three Share-sheet backgrounds and the Home screenshot with the latest weather layout. The photo-fit fixture uses a fixed non-zero Polaroid tilt so background Home-view rendering cannot change its screenshot. Geometry checks and exact screenshot comparisons remain enabled.

For future visual changes, use `npm run test:screens:docker:update`, review the generated references, and then verify with `npm run test:screens:docker`, as described in `frontend/tests/screens/README.md`.
