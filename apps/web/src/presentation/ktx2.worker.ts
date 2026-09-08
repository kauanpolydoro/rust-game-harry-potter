import * as decoder from '@babylonjs/ktx2decoder'
import { workerFunction } from '@babylonjs/core/Misc/khronosTextureContainer2Worker'
import astc from '@babylonjs/ktx2decoder/wasm/uastc_astc.wasm?url'
import bc7 from '@babylonjs/ktx2decoder/wasm/uastc_bc7.wasm?url'
import rgba from '@babylonjs/ktx2decoder/wasm/uastc_rgba8_unorm_v2.wasm?url'
import srgb from '@babylonjs/ktx2decoder/wasm/uastc_rgba8_srgb_v2.wasm?url'
import zstd from '@babylonjs/ktx2decoder/wasm/zstddec.wasm?url'

decoder.LiteTranscoder_UASTC_ASTC.WasmModuleURL = astc
decoder.LiteTranscoder_UASTC_BC7.WasmModuleURL = bc7
decoder.LiteTranscoder_UASTC_RGBA_UNORM.WasmModuleURL = rgba
decoder.LiteTranscoder_UASTC_RGBA_SRGB.WasmModuleURL = srgb
decoder.ZSTDDecoder.WasmModuleURL = zstd
workerFunction(decoder)
