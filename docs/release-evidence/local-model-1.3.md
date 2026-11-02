# Ask with a local model, item 1.3: measurements

M4 Pro, macOS 27.2. SmolLM3 3B Q4_K_M, llama.cpp `b11877` (`e321bdad`), a synthetic meeting transcript of about 4,000 tokens. No real meeting text.

| | |
|---|---|
| Helper size | 19.5 MB, stripped, no libraries outside the OS |
| Model size | 1.9 GB |
| Memory while loaded | 2.4 GB |
| Load time | 6 s |
| Reading speed | 410–750 tokens/s |
| Writing speed | 73–81 tokens/s |

The English answer was correct. The Spanish answer named the right person but added a date that wasn't in the transcript.

Not measured: an 8 GB Mac, and real meetings.

## Windows (W-1.3)

i9-13900H laptop, Windows 11, with an RTX 4080 Laptop GPU and Iris Xe. The same model and llama.cpp commit, from llama.cpp's own `llama-b11877-bin-win-vulkan-x64.zip` (SHA-256 checked), started with core's flags (`-ngl 99 -c 8192 --jinja --reasoning off`). One short question, about 30 tokens in and 60–75 out. No real meeting text.

| | RTX 4080 (Vulkan) | Iris Xe (Vulkan) | CPU only |
|---|---|---|---|
| Load time | 4.5 s | 8.9 s | 4.9 s |
| Reading speed | 1,236–1,347 tokens/s | 26 tokens/s | 46 tokens/s |
| Writing speed | 105–111 tokens/s | 17 tokens/s | 12 tokens/s |
| Peak working set | 2.5 GB | 2.9 GB | 3.7 GB |

With no device named, llama.cpp chose the RTX. All the answers were correct and plain.

| | |
|---|---|
| Helper | 25 files, 82.9 MB in `llama\`: the server and its DLLs, the 14 CPU backends ggml chooses between, the Vulkan backend |
| Installer | 84,745,444 bytes with the helper, 70,671,989 without: it adds 14.1 MB |
| Model download | 78 s here; SHA-256 matched |

Core's `local` streaming branch ran for the first time here: an ignored test (`local_model::tests::a_real_local_model_streams_an_answer`) streams through `api::inference` and `chat::forward` as Ask does, in 4.7 s from a cold start. Core's `a_real_helper_answers_and_stops` passes on Windows too.

A PC whose only GPU is integrated reads slowly: Iris Xe reads at 26 tokens/s, slower than the CPU. An Ask prompt with a few thousand tokens of context could wait a minute or more before the first word there.

Not measured on Windows: an 8 GB PC, a PC with no GPU driver, and real meetings.
