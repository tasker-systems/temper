import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { downloadFile } from "@huggingface/hub";
import { AutoTokenizer, type PreTrainedTokenizer, type Tensor } from "@huggingface/transformers";
// eslint-disable-next-line -- onnxruntime-node re-exports from onnxruntime-common.
// Vercel's TypeScript may not resolve the re-exported types; local typecheck uses
// onnxruntime-common directly via skipLibCheck.
import * as ort from "onnxruntime-node";

export const EMBEDDING_DIM = 768;

const MODEL_NAME = "BAAI/bge-base-en-v1.5";
const CACHE_DIR = join("/tmp", "temper-models", "bge-base-en-v1.5");
const ONNX_FILE = "onnx/model.onnx";

let _tokenizer: PreTrainedTokenizer | null = null;
let _session: ort.InferenceSession | null = null;

async function getTokenizer(): Promise<PreTrainedTokenizer> {
  if (!_tokenizer) {
    _tokenizer = await AutoTokenizer.from_pretrained(MODEL_NAME, {
      cache_dir: CACHE_DIR,
    });
  }
  return _tokenizer;
}

/**
 * Download the ONNX model file from HuggingFace Hub if not already cached.
 * Uses @huggingface/hub for reliable downloading with proper caching,
 * matching the Rust crate's hf_hub approach.
 */
async function ensureModel(): Promise<string> {
  const modelPath = join(CACHE_DIR, ONNX_FILE);
  const modelDir = join(CACHE_DIR, "onnx");

  if (existsSync(modelPath)) {
    return modelPath;
  }

  mkdirSync(modelDir, { recursive: true });

  const response = await downloadFile({
    repo: MODEL_NAME,
    path: ONNX_FILE,
  });

  if (!response) {
    throw new Error(`Failed to download model from ${MODEL_NAME}/${ONNX_FILE}`);
  }

  const buffer = Buffer.from(await response.arrayBuffer());
  writeFileSync(modelPath, buffer);

  return modelPath;
}

async function getSession(): Promise<ort.InferenceSession> {
  if (!_session) {
    const modelPath = await ensureModel();
    _session = await ort.InferenceSession.create(modelPath, {
      executionProviders: ["cpu"],
    });
  }
  return _session;
}

/**
 * Mean pooling: average token embeddings, respecting the attention mask.
 * Shape of lastHiddenState: [batch, seq_len, hidden_dim]
 * Shape of attentionMask:   [batch, seq_len]
 */
function meanPool(
  lastHiddenState: Float32Array,
  attentionMask: BigInt64Array,
  batchSize: number,
  seqLen: number,
  hiddenDim: number,
): Float32Array {
  const result = new Float32Array(batchSize * hiddenDim);

  for (let b = 0; b < batchSize; b++) {
    let maskSum = 0;
    for (let s = 0; s < seqLen; s++) {
      const maskVal = Number(attentionMask[b * seqLen + s]);
      maskSum += maskVal;
      for (let d = 0; d < hiddenDim; d++) {
        result[b * hiddenDim + d] +=
          lastHiddenState[b * seqLen * hiddenDim + s * hiddenDim + d] * maskVal;
      }
    }
    // Divide by mask sum (number of real tokens)
    if (maskSum > 0) {
      for (let d = 0; d < hiddenDim; d++) {
        result[b * hiddenDim + d] /= maskSum;
      }
    }
  }

  return result;
}

/**
 * L2 normalize each vector in the batch.
 */
function l2Normalize(vectors: Float32Array, batchSize: number, dim: number): Float32Array {
  for (let b = 0; b < batchSize; b++) {
    let norm = 0;
    const offset = b * dim;
    for (let d = 0; d < dim; d++) {
      norm += vectors[offset + d] * vectors[offset + d];
    }
    norm = Math.sqrt(norm);
    if (norm > 0) {
      for (let d = 0; d < dim; d++) {
        vectors[offset + d] /= norm;
      }
    }
  }
  return vectors;
}

/**
 * Embed an array of texts using bge-base-en-v1.5, producing 768-dimensional
 * L2-normalized vectors suitable for cosine similarity search.
 */
export async function embedTexts(texts: string[]): Promise<number[][]> {
  if (texts.length === 0) return [];

  const tokenizer = await getTokenizer();
  const session = await getSession();

  // Tokenize all texts as a batch
  const encoded = tokenizer(texts, {
    padding: true,
    truncation: true,
    max_length: 512,
    return_tensors: "np",
  }) as { input_ids: Tensor; attention_mask: Tensor; token_type_ids?: Tensor };

  const inputIds = encoded.input_ids;
  const attentionMask = encoded.attention_mask;

  const batchSize = texts.length;
  const seqLen = (inputIds.dims as number[])[1];

  // Build ONNX tensors — convert tokenizer output to BigInt64Array for onnxruntime
  const toBigInt64 = (data: Tensor["data"]): BigInt64Array => {
    const arr = new BigInt64Array(data.length);
    for (let i = 0; i < data.length; i++) {
      arr[i] = BigInt(data[i] as number);
    }
    return arr;
  };

  const inputIdsTensor = new ort.Tensor("int64", toBigInt64(inputIds.data), [batchSize, seqLen]);
  const attentionMaskTensor = new ort.Tensor("int64", toBigInt64(attentionMask.data), [
    batchSize,
    seqLen,
  ]);
  const tokenTypeIdsTensor = new ort.Tensor(
    "int64",
    new BigInt64Array(batchSize * seqLen), // zeros
    [batchSize, seqLen],
  );

  // Run inference
  const feeds: Record<string, ort.Tensor> = {
    input_ids: inputIdsTensor,
    attention_mask: attentionMaskTensor,
    token_type_ids: tokenTypeIdsTensor,
  };

  const output = await session.run(feeds);

  // The model output key is typically "last_hidden_state"
  const outputKey = Object.keys(output)[0];
  const lastHiddenState = output[outputKey].data as Float32Array;
  const hiddenDim = (output[outputKey].dims as number[])[2];

  if (hiddenDim !== EMBEDDING_DIM) {
    throw new Error(`Expected embedding dim ${EMBEDDING_DIM}, got ${hiddenDim}`);
  }

  // Mean pool and normalize
  const pooled = meanPool(
    lastHiddenState,
    attentionMaskTensor.data as BigInt64Array,
    batchSize,
    seqLen,
    hiddenDim,
  );
  const normalized = l2Normalize(pooled, batchSize, hiddenDim);

  // Split into individual vectors
  const result: number[][] = [];
  for (let b = 0; b < batchSize; b++) {
    result.push(Array.from(normalized.slice(b * hiddenDim, (b + 1) * hiddenDim)));
  }

  return result;
}
