# AGENTS.md — Psychotalk

## Aprovação e registro

- Antes de implementar mudanças não triviais, apresente uma proposta e aguarde a aprovação do usuário.
- Registre mudanças relevantes no `CHANGELOG.md`.

## Sincronização com o GitHub

- Após concluir cada alteração aprovada pelo usuário e executar as verificações pertinentes, faça commit e push dos arquivos dessa alteração para o repositório `https://github.com/fpoletto/Psychotalk`, normalmente em `main`.
- A aprovação da alteração também autoriza sua sincronização; não peça uma segunda aprovação somente para fazer commit ou push.
- Confira se o commit enviado corresponde ao commit local e informe qualquer impedimento de sincronização, sem declarar o GitHub atualizado antes da verificação.
- Preserve alterações locais alheias à tarefa e a visibilidade autorizada pelo usuário para o repositório. Não envie credenciais, cofres ou ferramentas locais.
- Não faça force push nem sobrescreva alterações remotas. Se houver divergência, investigue e preserve o trabalho existente antes de sincronizar.

## Privacidade na publicação

- Use identidade de commit sem e-mail pessoal; configure somente este checkout, sem alterar a configuração global ou outros projetos.
- Antes de publicar o repositório, confira também metadados do histórico e caminhos locais embutidos nos binários; a revisão apenas da árvore atual não basta.
- Não mude a visibilidade de outros repositórios sem instrução explícita para cada escopo.

- O repositório público contém somente o histórico limpo. Não envie objetos ou tags do arquivo privado para este remoto.
