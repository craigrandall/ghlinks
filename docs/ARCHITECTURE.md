# ghlinks Architectural Diagrams

**Mermaid diagrams are the canonical sources**; using `mermaid-cli` or an online Mermaid renderer, you can produce both SVG and PNG artifacts from each.

Diagram labels are "Mermaid‑safe" (no HTML tags), using `\n` for multi-line text.

## 1. Architecture diagram (canonical)

```mermaid
flowchart TD
    subgraph Input
        A["Input file\n(list of URLs)"]
    end

    subgraph Classification
        B["classify.rs\nURL → LinkKind"]
    end

    subgraph GitHub
        C["github.rs\nGitHub client\nrepo/gist/pages metadata"]
    end

    subgraph Discovery
        D["discovery.rs\nHacker News\nexternal mentions"]
    end

    subgraph Model
        E["model.rs\nRepoData, GistData,\nReleaseEntry, ExternalMention"]
    end

    subgraph Orchestration
        F["main.rs\nCLI, concurrency,\nJSON output"]
    end

    A --> B
    B --> C
    B --> D
    C --> E
    D --> E
    E --> F
```

This version contains:
- No HTML tags  
- No unsupported characters  
- Only Mermaid‑safe multi‑line labels  

**The Mermaid diagram above (`flowchart TD`) is the canonical source**; using `mermaid-cli` or an online Mermaid renderer, you can produce both SVG and PNG artifacts from it.

---

*ASCII architecture diagram (quick visual)*

```text
                +------------------+
                |   Input File     |
                |  (list of URLs)  |
                +---------+--------+
                          |
                          v
                +------------------+
                |    classify.rs   |
                |  URL → LinkKind  |
                +---------+--------+
                          |
          +---------------+----------------+
          |                                |
          v                                v
+------------------+             +------------------+
|    github.rs     |             |  discovery.rs    |
| GitHub metadata  |             | External mentions|
+--------+---------+             +--------+---------+
         |                                |
         +---------------+----------------+
                         |
                         v
                +------------------+
                |    model.rs      |
                |  Structured data |
                +---------+--------+
                          |
                          v
                +------------------+
                |    main.rs       |
                | JSON output file |
                +------------------+
```

---

## 2. Sequence diagram (end‑to‑end run)

```mermaid
sequenceDiagram
    participant User
    participant Main as main.rs
    participant Classify as classify.rs
    participant GitHub as github.rs
    participant Discovery as discovery.rs
    participant Model as model.rs

    User->>Main: Run ghlinks\n--input links.txt\n--output report.json
    Main->>Main: Read links.txt
    Main->>Classify: classify(urls)
    Classify-->>Main: LinkKind list

    loop For each LinkKind
        Main->>GitHub: fetch metadata\n(repos/gists/pages)
        GitHub-->>Main: RepoData/GistData

        alt external discovery enabled
            Main->>Discovery: fetch external mentions\n(Hacker News)
            Discovery-->>Main: ExternalMention list
        else external discovery disabled
            Main-->>Main: Skip discovery
        end

        Main->>Model: assemble structs\n(RepoData/GistData,\nExternalMention)
        Model-->>Main: AnalysisOutput
    end

    Main->>Main: Serialize AnalysisOutput\nas JSON
    Main-->>User: report.json
```

---

*ASCII sequence / data flow diagram (quick visual)*

```text
User
 |
 | run ghlinks --input links.txt --output report.json
 v
main.rs
 |
 | read links.txt
 v
classify.rs
 |
 | classify each URL → LinkKind
 v
main.rs
 |
 | spawn concurrent tasks
 v
github.rs ----------------------+
 |                              |
 | fetch metadata               |
 | fetch releases               |
 | fetch languages              |
 | fetch contributors           |
 +------------------------------+
 |
 v
discovery.rs (optional)
 |
 | fetch external mentions
 v
main.rs
 |
 | assemble model structs
 v
model.rs
 |
 | serialize to JSON
 v
report.json
```

---

## 3. Class diagram for Rust modules (conceptual)

```mermaid
classDiagram
    class LinkKind {
        <<enum>>
        +RepoRoot
        +RepoFile
        +Gist
        +PagesSite
    }

    class Classify {
        +classify(url: String): LinkKind
    }

    class GitHub {
        -client: reqwest::Client
        -token: Option<String>
        +new(client: Client, token: Option<String>): GitHub
        +fetch_repo(owner: &str, repo: &str): Result<RepoData>
        +fetch_gist(id: &str): Result<GistData>
        +fetch_languages(owner: &str, repo: &str): Result<LanguageMap>
        +fetch_releases(owner: &str, repo: &str): Result<Vec<ReleaseEntry>>
    }

    class Discovery {
        +fetch_mentions(repo: &RepoData): Result<Vec<ExternalMention>>
    }

    class RepoData {
        <<struct>>
        +owner: String
        +name: String
        +description: Option<String>
        +license: LicenseInfo
        +releases: Vec<ReleaseEntry>
        +languages: LanguageMap
        +community: CommunityStats
        +external: Vec<ExternalMention>
    }

    class GistData {
        <<struct>>
        +owner: String
        +id: String
        +description: Option<String>
        +files: Vec<GistFile>
    }

    class ExternalMention {
        <<struct>>
        +source_url: String
        +title: Option<String>
        +score: Option<i64>
        +comments: Option<i64>
    }

    class Main {
        +run(): Result<()>
    }

    Classify --> LinkKind
    Main --> Classify
    Main --> GitHub
    Main --> Discovery
    GitHub --> RepoData
    GitHub --> GistData
    Discovery --> ExternalMention
    RepoData --> ExternalMention
```

---

## 4. Pipeline diagram showing concurrency and data flow

```mermaid
flowchart TD
    A["Input file\nlinks.txt"] --> B["main.rs\nread URLs"]

    B --> C["classify.rs\nURL → LinkKind list"]

    subgraph Concurrent processing
        direction LR
        D["Per-link task\nspawned via futures::stream"]
        D --> E["github.rs\nfetch repo/gist/pages\nmetadata, releases,\nlanguages, contributors"]
        D --> F["discovery.rs\nfetch external mentions\n(Hacker News)"]
    end

    C --> D

    E --> G["model.rs\nassemble RepoData/GistData,\nExternalMention"]
    F --> G

    G --> H["main.rs\naggregate AnalysisOutput"]
    H --> I["Serialize to JSON\nreport.json"]
```

---

You can put each diagram in its own `.mmd` file (e.g., `architecture.mmd`, `sequence.mmd`, `classes.mmd`, `pipeline.mmd`) and render them with `mmdc` to PNG/SVG as needed.

For example, the following will render cleanly:

```
mmdc -i architecture.mmd -o architecture.png
mmdc -i architecture.mmd -o architecture.svg
```
