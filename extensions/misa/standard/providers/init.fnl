;; Stock API-key and OAuth providers. Providers that exist only for tests or
;; custom configuration (command, fake, generic, openai-compatible,
;; openai-options) are deliberately not part of this directory's module.
(local anthropic (require :misa.standard.providers.anthropic))
(local auth (require :misa.standard.providers.auth))
(local cerebras (require :misa.standard.providers.cerebras))
(local claude (require :misa.standard.providers.claude))
(local deepinfra (require :misa.standard.providers.deepinfra))
(local deepseek (require :misa.standard.providers.deepseek))
(local fireworks (require :misa.standard.providers.fireworks))
(local groq (require :misa.standard.providers.groq))
(local huggingface (require :misa.standard.providers.huggingface))
(local kimi (require :misa.standard.providers.kimi))
(local mistral (require :misa.standard.providers.mistral))
(local moonshot (require :misa.standard.providers.moonshot))
(local novita (require :misa.standard.providers.novita))
(local nvidia (require :misa.standard.providers.nvidia))
(local openai (require :misa.standard.providers.openai))
(local openai-codex (require :misa.standard.providers.openai-codex))
(local openrouter (require :misa.standard.providers.openrouter))
(local siliconflow (require :misa.standard.providers.siliconflow))
(local together (require :misa.standard.providers.together))
(local venice (require :misa.standard.providers.venice))
(local xai (require :misa.standard.providers.xai))

(misa.merge-definitions [anthropic
                         auth
                         cerebras
                         claude
                         deepinfra
                         deepseek
                         fireworks
                         groq
                         huggingface
                         kimi
                         mistral
                         moonshot
                         novita
                         nvidia
                         openai
                         openai-codex
                         openrouter
                         siliconflow
                         together
                         venice
                         xai])
