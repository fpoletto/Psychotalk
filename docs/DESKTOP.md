# Psychotalk desktop experimental 0.4.2

Interface no navegador, núcleo Rust e cofre SQLCipher locais. Pacotes para macOS arm64/Intel (validados em macOS 15) e Linux x86_64 (validado em Ubuntu 22.04 com Secret Service). Outras distribuições compatíveis, incluindo Debian, não foram executadas nesta entrega. O instalador atual exige GitHub CLI autenticado, mesmo com o repositório público.

## Instalar

```sh
gh auth login
psychotalk_install_dir="$(mktemp -d)"
gh release download v0.4.2-experimental --repo fpoletto/Psychotalk --pattern install.sh --dir "$psychotalk_install_dir" && sh "$psychotalk_install_dir/install.sh"
"$HOME/.local/bin/psychotalk"
```

O script baixa o pacote da sua arquitetura, verifica SHA-256 e instala no diretório do usuário. Rust, Node e Docker são ferramentas de desenvolvimento, não dependências de uso. O Tor e suas bibliotecas acompanham o pacote.

## Usar

O comando abre o navegador em um endereço 127.0.0.1 com autorização temporária, retirada da barra de endereço assim que a interface carrega. Não encaminhe essa URL. Recarregar retoma a interface e volta à tela de senha, sem executar novamente o comando, enquanto o comprovante for válido. O cofre é fechado antes de emitir a nova autorização; a senha é necessária para reabrir. Executar o comando novamente bloqueia a sessão anterior e abre uma janela autorizada. Não exponha a porta na rede.

A retomada guarda somente um comprovante aleatório de uso único em `sessionStorage`. O token de comandos continua apenas em memória, e senhas/chaves/histórico não são escritos nesse armazenamento. O navegador pode restaurar o comprovante junto com uma aba; ele só vale para o processo local atual e expira após oito horas sem requisições válidas. Nova execução pelo launcher, encerramento/reinício ou limpeza concluída invalidam os comprovantes. Uma aba duplicada que retoma assume o controle e bloqueia a anterior; não há duas páginas autorizadas ao mesmo tempo. Se o navegador negar esse armazenamento, a interface funciona na abertura pelo launcher e informa que recarregar exigirá abri-la novamente.

Configure senhas normal e de limpeza diferentes. A senha normal inicia Tor e abre o histórico. A senha de limpeza destrói somente o cofre desta instalação e invalida sua chave no chaveiro; não apaga cópias em outros participantes e pode destruir pedidos de exclusão pendentes. Após a limpeza, abra novamente pelo launcher para configurar outro cofre. Sem recuperação de senha.

Cada instalação tem identidade/código próprios; não há espelhamento ou importação automática do Android. Troque os códigos e confirme as impressões por um canal confiável. As mesmas regras de conversas individuais, grupos fixos de 3 a 5 identidades e exclusão por autor do README se aplicam. Android 0.3.0 é compatível com o protocolo.

Ao ocultar a aba, minimizar ou perder heartbeat, a sessão deve bloquear e encerrar Tor. Fechar o navegador não é uma notificação confiável: o watchdog nativo revoga a sessão sem heartbeat. Retomar requer senha. O processo HTTP local pode permanecer aberto e bloqueado; `psychotalk stop` ou Ctrl+C encerra o programa.

## Atualizar e remover

```sh
psychotalk update
psychotalk uninstall
```

Atualizar baixa a release desktop mais recente, incluindo experimentais, verifica o pacote, encerra a sessão e troca a versão do programa. O cofre é preservado. Desinstalar também preserva o cofre e sua credencial; para limpar seus dados, use a senha de limpeza antes de remover o programa. Não é prometido downgrade de schemas futuros.

macOS: dados em `~/Library/Application Support/Psychotalk`; proteção adicional pelo Keychain. O macOS pode pedir autorização de acesso ao chaveiro ao atualizar o executável; a distribuição inicial usa assinatura ad-hoc, sem notarização Apple. Linux: dados em `${XDG_DATA_HOME:-~/.local/share}/psychotalk`; exige um Secret Service desbloqueado (por exemplo GNOME Keyring). Se o chaveiro estiver indisponível, o programa não grava a chave em arquivo para contornar o erro. A proteção do chaveiro não é apresentada como garantia de hardware.

Opções avançadas: `--data-dir CAMINHO_ABSOLUTO`, `--tor CAMINHO` e `--no-open`. O último exibe no terminal uma URL de autorização temporária para abertura manual; trate-a como credencial da sessão do programa. `PSYCHOTALK_DATA_DIR` altera o diretório de dados e instalação; `PSYCHOTALK_BIN_DIR` altera o local do launcher. Guarde os mesmos valores ao atualizar/remover.

## Limites

Experimental, sem auditoria independente. O navegador, extensões e sistema operacional podem reter cópias ou capturar conteúdo; não há equivalente garantido ao bloqueio de capturas do Android. O aplicativo não persiste histórico/senhas em localStorage, IndexedDB ou cache de service worker. Não há garantia forense sobre RAM, backups do sistema ou flash. ACK de exclusão é declaração do aplicativo autenticado, não prova independente contra clientes modificados. O download autenticado no GitHub não ocorre pelo Tor do aplicativo; somente o tráfego entre participantes usa esse transporte.

## Compilar e testar como desenvolvedor

Exige Rust 1.100.0, compilador C, Python 3, OpenSSL e libevent de desenvolvimento. Em Linux, também patchelf; Node e Playwright são usados somente para os testes de navegador. Confira os workflows em `.github/workflows/` para os comandos por plataforma. Neste Mac, execute `. scripts/env.sh` antes dos comandos Rust para usar as ferramentas locais e o SDK já preparado.

```sh
sh scripts/build-desktop-tor.sh
cargo test --workspace
PSYCHOTALK_TEST_KEYRING=1 cargo test -p psychotalk-desktop --test os_keyring
python3 scripts/private-build.py cargo build --locked --release -p psychotalk-desktop --bin psychotalk
npm ci
npx playwright install chromium
PSYCHOTALK_TEST_BINARY=target/release/psychotalk PSYCHOTALK_TEST_TOR=.tools/desktop-tor/tor-0.4.9.14/src/app/tor npm run test:web
python3 scripts/package-desktop.py
python3 scripts/test-desktop-installer.py dist/psychotalk-0.4.2-experimental-macos-arm64.tar.gz
```

Ajuste o nome do pacote no último comando para a plataforma. Os testes usam cofres e credenciais descartáveis separados do perfil normal. O teste de chaveiro somente executa operações reais com a variável explícita; sem ela retorna sem consultar/gravar o serviço.

Para a prova com o Android, compile o exemplo `android_probe` com `--features psychotalk-core/probe-diagnostics`, gere o APK de instrumentação e use o emulador arm64 com o APK principal 0.3.1. Exemplo, depois de preparar o SDK/AVD:

```sh
cargo build -p psychotalk-desktop --features psychotalk-core/probe-diagnostics --example android_probe
python3 scripts/test-desktop-android.py --serial emulator-5554 --tor .tools/desktop-tor/tor-0.4.9.14/src/app/tor --root .tools/interop-novo --groups
```

O diretório da fixture deve ser novo. `--cache DIRETÓRIO` reutiliza somente arquivos públicos `cached-*`, sem guards, estado ou identidades. ADB transfere códigos/convites públicos e sinalizadores de progresso de teste; mensagens e confirmações passam pelo Tor/mTLS. O driver confirma épocas de descarte de UI; não equivale ao fluxo completo por toques na interface Android. Não rode a instrumentação em um aparelho pessoal sem reservar a sessão para teste.
