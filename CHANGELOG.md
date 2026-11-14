# Changelog

Formato [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Unreleased]

### Changed
- README translated into English and expanded with desktop 0.4.2 resume behavior, security benefits/boundaries, platform requirements, and current validation limits.
- Public distribution prepared from a clean source history; original repository retained privately. Commit authorship uses a non-personal address.
- Rust release builds remap workstation source/home paths; vendored OpenSSL and desktop Tor build in anonymous temporary roots. APKs, signing/archive bytes and desktop archives are checked before publication.

## [0.3.1-experimental] — 2026-11-14

### Fixed
- Android privacy rebuild removes local build paths and increments versionCode to 6; protocol and vault format are unchanged and the existing debug signing certificate is retained.

## [0.4.2-experimental] — 2026-11-14

### Added
- Retomada da interface ao recarregar por comprovante de uso único em sessionStorage, separado do token de comandos mantido em memória e vinculado a cada documento.
- Expiração nativa após oito horas de inatividade, invalidação no reinício/launcher/limpeza e fallback explícito quando o navegador nega sessionStorage.
- Testes de recarregamentos repetidos, duplicação de aba, pedidos antigos, resposta de desbloqueio atrasada, reinício, limpeza e relógios nativos de expiração.

### Changed
- Cada retomada revoga a autorização anterior, fecha o cofre e troca as credenciais antes de mostrar a tela de senha; a senha normal continua necessária.
- Operações da API revalidam a geração sob exclusão mútua antes de executar; requisições e respostas da página anterior não podem atuar na nova sessão.
- Limpeza local concluída invalida também a retomada, exigindo abrir pelo launcher para configurar um novo cofre.

## [0.4.1-experimental] — 2026-11-14

### Fixed
- Botão de criação de grupo recalculado quando um novo snapshot fornece a identidade Tor, preservando o nome e os participantes do formulário aberto.
- Regressão de navegador com entrega atrasada do código Tor real: formulário válido permanece bloqueado enquanto aguarda e habilita sem editar campos ou reabrir o diálogo.

## [0.4.0-experimental] — 2026-11-14

### Added
- Desktop 0.4.0 experimental: núcleo local com interface no navegador, cofre SQLCipher protegido pelo chaveiro do sistema e instalador autenticado pelo GitHub.
- Pacotes macOS Apple Silicon/Intel e Linux x86_64 com Tor 0.4.9.14, bibliotecas e licenças empacotadas; instalação/atualização/remoção preservando o cofre e rejeição de checksum incorreto.
- Validação com 39 testes Rust efetivos, navegador/cofre real nas três plataformas, testes dos binários distribuídos e grupo misto de dois desktops/Android com ausência e reconexão do destinatário.

### Changed
- Validação local complementar do pacote publicado após desbloquear o Mac: cofre/Keychain, reabertura, watchdog e limpeza passaram ao aguardar o endereço Tor antes de abrir o formulário de grupo. A suíte original expôs o botão de criação desabilitado após preparação tardia da identidade; correção pendente naquela verificação, resolvida em 0.4.1.

## [0.3.0-experimental]

### Added
- Repositório privado GitHub com versionamento inicial do MVP e distribuição do APK 0.3.0/checksum pela release privada.
- APK 0.3.0: grupos fixos de 3 a 5 identidades, criação por contatos confirmados e entrada com conferência explícita do convite/composição.
- Formação com aceitação e prontidão autenticadas de todos; convite público sem chaves privadas, IDs imutáveis e importação idempotente.
- Fanout direto autor→destinatário por Tor/mTLS, entrega por destinatário e exclusão somente do próprio autor, com confirmações independentes persistidas.
- Interface com autor/tagname local, contagem de entregas e exclusões, detalhes por destinatário, edição de tagnames dos membros e cópia de convite.

### Changed
- Diretriz de trabalho registrada em `AGENTS.md`: concluir, validar e sincronizar no GitHub cada alteração aprovada pelo usuário.
- Schema cifrado 3 com migração transacional de 2, preservando conversas, identidade, filas e pendências anteriores.
- Multiplexação de grupos e conversas individuais por par, negociação TLS ALPN e compatibilidade com wire v1; convite não aceito adia seu fluxo sem bloquear conversas existentes.
- Participar de grupo autoriza somente seu tráfego; conversas individuais com os membros exigem pareamento separado.
- Validação com grupos de 3 e 5 clientes Tor reais, compatibilidade com cliente antigo, atualização Android 0.2.0→0.3.0, 32 testes Rust e 8 casos Android efetivos.

## [0.2.0] — 2026-11-13

### Changed
- APK 0.2.0: lista de conversas individuais, adição de vários contatos e edição de tagnames locais cifrados.
- Migração transacional do cofre 0.1.x, preservando identidade, mensagens, sequências, marcadores e pendências.
- Exclusão limitada à conversa selecionada; senha de limpeza permanece global para o cofre local.
- Transporte com até quatro conexões, autenticação por allowlist de certificados verificados e coordenação de envio/exclusão por conversa.
- Validação com três clientes Tor reais, compatibilidade wire v1 com peer 0.1.2 e atualização do APK antigo no emulador.

- APK 0.1.2: mensagens próprias alinhadas ao final da linha e mensagens do contato ao início, com recuo lateral de 48 dp, largura máxima de 320 dp e cores semânticas distintas. Identificação de autoria e estado de entrega preservados.

### Fixed
- Ícone adaptativo e nome explícito da atividade de abertura para identificar o Psychotalk na lista de aplicativos. APK 0.1.1 com o mesmo pacote e assinatura de teste para atualização sem desinstalar.

### Added
- Aplicativo Android 10+ arm64 em Kotlin/Compose e núcleo Rust com bindings JNI mínimos.
- Transporte onion v3 com Tor embarcado, controle autenticado, TLS 1.3 mútuo e pins verificados; sem transporte alternativo.
- Cofre SQLCipher real com chave binária de 256 bits, Argon2id, envelopes distintos e Android Keystore.
- Configuração de duas senhas, limpeza local com intenção durável e recuperação antes da rede.
- Fila cifrada, persistência antes do ACK, sequências monotônicas e deduplicação.
- Exclusão por autoria com marcadores persistentes, confirmação do contato e prioridade na reconexão.
- Bloqueio de sessão ao perder primeiro plano e watchdog de heartbeat.
- Testes adversariais TLS, testes reais SQLCipher/protocolo, clientes Tor reais e testes instrumentados Android.
- Build local, APK experimental, inventário de dependências e relatório de validação.
