import {
  createConnection,
  ProposedFeatures,
  Range,
  TextDocuments,
  TextDocumentSyncKind,
  TextEdit,
  type DocumentFormattingParams,
  type InitializeParams,
  type InitializeResult,
} from "vscode-languageserver/node";
import { TextDocument } from "vscode-languageserver-textdocument";
import { formatSource, type FormatResult } from "./cfmt";

type Settings = { readonly path: string; readonly width: number };

const DEFAULT_SETTINGS: Settings = { path: "cfmt", width: 100 };

/// Read settings from a client's untyped `initializationOptions`, falling back per field.
const readSettings = (options: unknown): Settings => {
  const given = (options ?? {}) as Partial<Record<keyof Settings, unknown>>;
  return {
    path:
      typeof given.path === "string" && given.path.length > 0
        ? given.path
        : DEFAULT_SETTINGS.path,
    width:
      typeof given.width === "number" && Number.isFinite(given.width)
        ? given.width
        : DEFAULT_SETTINGS.width,
  };
};

const connection = createConnection(ProposedFeatures.all);
const documents = new TextDocuments(TextDocument);
let settings: Settings = DEFAULT_SETTINGS;

const wholeDocument = (doc: TextDocument): Range =>
  Range.create(doc.positionAt(0), doc.positionAt(doc.getText().length));

/// One full-document edit when cfmt changed the text; nothing when unchanged; an error toast
/// when cfmt failed.
const toEdits = (doc: TextDocument, result: FormatResult): readonly TextEdit[] => {
  switch (result.kind) {
    case "formatted":
      return result.text === doc.getText()
        ? []
        : [TextEdit.replace(wholeDocument(doc), result.text)];
    case "failed":
      connection.window.showErrorMessage(`cfmt: ${result.message}`);
      return [];
  }
};

connection.onInitialize((params: InitializeParams): InitializeResult => {
  settings = readSettings(params.initializationOptions);
  return {
    capabilities: {
      textDocumentSync: TextDocumentSyncKind.Incremental,
      documentFormattingProvider: true,
    },
  };
});

connection.onDocumentFormatting(
  async (params: DocumentFormattingParams): Promise<TextEdit[]> => {
    const doc = documents.get(params.textDocument.uri);
    return doc === undefined
      ? []
      : [...toEdits(doc, await formatSource(settings.path, settings.width, doc.getText()))];
  },
);

documents.listen(connection);
connection.listen();
