// @hot-labs/near-connect 0.12.0, wrapped by web/tools/bundle-near-connect.mjs. Do not edit.
const defs = {};
const cache = {};
function load(id) {
  if (cache[id]) return cache[id].exports;
  const module = (cache[id] = { exports: {} });
  defs[id](module, module.exports, (spec) => load(resolve(id, spec)));
  return module.exports;
}
function resolve(from, spec) {
  const parts = (from.split("/").slice(0, -1).join("/") + "/" + spec).split("/");
  const stack = [];
  for (const p of parts) { if (p === "..") stack.pop(); else if (p && p !== ".") stack.push(p); }
  const base = stack.join("/");
  if (defs[base + ".js"]) return base + ".js";
  if (defs[base + "/index.js"]) return base + "/index.js";
  throw new Error("near-connect bundle: cannot resolve " + spec + " from " + from);
}
defs["InjectedWallet.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.InjectedWallet = void 0;
const actions_1 = require("./actions");
const gas_keys_1 = require("./actions/gas-keys");
class InjectedWallet {
    connector;
    wallet;
    constructor(connector, wallet) {
        this.connector = connector;
        this.wallet = wallet;
    }
    get manifest() {
        return this.wallet.manifest;
    }
    async signIn({ addFunctionCallKey, network }) {
        return this.wallet.signIn({
            network: network ?? this.connector.network,
            addFunctionCallKey,
        });
    }
    async signInAndSignMessage(data) {
        return this.wallet.signInAndSignMessage({
            network: data?.network ?? this.connector.network,
            addFunctionCallKey: data.addFunctionCallKey,
            messageParams: data.messageParams,
        });
    }
    async signOut(data) {
        await this.wallet.signOut({ network: data?.network ?? this.connector.network });
    }
    async getAccounts(data) {
        return this.wallet.getAccounts({ network: data?.network ?? this.connector.network });
    }
    async signAndSendTransaction(params) {
        const actions = (0, actions_1.nearActionsToConnectorActions)(params.actions);
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, actions, this.manifest.name);
        const network = params.network ?? this.connector.network;
        const result = await this.wallet.signAndSendTransaction({ ...params, actions, network });
        if (!result)
            throw new Error("No result from wallet");
        // @ts-ignore
        if (Array.isArray(result.transactions))
            return result.transactions[0];
        return result;
    }
    async signAndSendTransactions(params) {
        const network = params.network ?? this.connector.network;
        const transactions = params.transactions.map((transaction) => ({
            actions: (0, actions_1.nearActionsToConnectorActions)(transaction.actions),
            receiverId: transaction.receiverId,
        }));
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, transactions.flatMap((transaction) => transaction.actions), this.manifest.name);
        const result = await this.wallet.signAndSendTransactions({ ...params, transactions, network });
        if (!result)
            throw new Error("No result from wallet");
        // @ts-ignore
        if (Array.isArray(result.transactions))
            return result.transactions;
        return result;
    }
    async signMessage(params) {
        return this.wallet.signMessage({ ...params, network: params.network ?? this.connector.network });
    }
    async signDelegateActions(params) {
        const delegateActions = params.delegateActions.map((delegateAction) => ({
            ...delegateAction,
            actions: (0, actions_1.nearActionsToConnectorActions)(delegateAction.actions),
        }));
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, delegateActions.flatMap((delegateAction) => delegateAction.actions), this.manifest.name);
        return this.wallet.signDelegateActions({
            ...params,
            delegateActions,
            network: params.network ?? this.connector.network,
        });
    }
}
exports.InjectedWallet = InjectedWallet;
};
defs["NearConnector.js"] = function (module, exports, require) {
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.NearConnector = void 0;
const events_1 = require("./helpers/events");
const NearWalletsPopup_1 = require("./popups/NearWalletsPopup");
const storage_1 = require("./helpers/storage");
const indexdb_1 = __importDefault(require("./helpers/indexdb"));
const ParentFrameWallet_1 = require("./ParentFrameWallet");
const InjectedWallet_1 = require("./InjectedWallet");
const SandboxedWallet_1 = require("./SandboxedWallet");
const defaultManifests = [
    "https://raw.githubusercontent.com/hot-dao/near-selector/refs/heads/main/repository/manifest.json",
    "https://cdn.jsdelivr.net/gh/azbang/hot-connector/repository/manifest.json",
];
function createFilterForWalletFeatures(features) {
    return (wallet) => {
        if (Object.entries(features).length === 0)
            return true;
        return Object.entries(features)
            .filter(([_, value]) => value === true)
            .every(([key]) => {
            return wallet.manifest.features?.[key] === true;
        });
    };
}
class NearConnector {
    storage;
    events;
    db;
    logger;
    wallets = [];
    manifest = { wallets: [], version: "1.0.0" };
    features = {};
    network = "mainnet";
    providers = { mainnet: [], testnet: [] };
    walletConnect;
    footerBranding;
    excludedWallets = [];
    autoConnect;
    cspNonce;
    whenManifestLoaded;
    constructor(options) {
        this.db = new indexdb_1.default("hot-connector", "wallets");
        this.storage = options?.storage ?? new storage_1.LocalStorage();
        this.events = options?.events ?? new events_1.EventEmitter();
        this.logger = options?.logger;
        this.cspNonce = options?.cspNonce;
        this.network = options?.network ?? "mainnet";
        this.walletConnect = options?.walletConnect;
        this.autoConnect = options?.autoConnect ?? true;
        this.providers = options?.providers ?? { mainnet: [], testnet: [] };
        this.excludedWallets = options?.excludedWallets ?? [];
        this.features = options?.features ?? {};
        if (options?.footerBranding !== undefined) {
            this.footerBranding = options?.footerBranding;
        }
        else {
            this.footerBranding = {
                icon: "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20fill%3D%22none%22%20viewBox%3D%220%200%20512%20512%22%20class%3D%22size-7%22%3E%3Crect%20width%3D%22512%22%20height%3D%22512%22%20fill%3D%22%2300ec97%22%20rx%3D%22110%22%2F%3E%3Cpath%20fill%3D%22%23000%22%20d%3D%22M373.89%20106.199a31.95%2031.95%200%200%200-27.213%2015.207l-62.631%2092.979a6.67%206.67%200%200%200-.989%205.001%206.66%206.66%200%200%200%202.837%204.235%206.67%206.67%200%200%200%208.032-.494l61.643-53.47c1.02-.924%202.599-.827%203.523.193.419.473.644%201.074.644%201.697v167.402a2.49%202.49%200%200%201-2.502%202.491%202.48%202.48%200%200%201-1.912-.891L168.976%20117.497a31.93%2031.93%200%200%200-24.356-11.298h-6.508c-17.623%200-31.917%2014.294-31.917%2031.917v235.767c0%2017.623%2014.294%2031.917%2031.917%2031.917a31.94%2031.94%200%200%200%2027.213-15.206l62.631-92.98a6.66%206.66%200%200%200-1.847-9.236%206.67%206.67%200%200%200-8.033.494l-61.643%2053.471c-1.02.923-2.599.826-3.522-.194a2.5%202.5%200%200%201-.634-1.697V173.008a2.49%202.49%200%200%201%202.502-2.492c.731%200%201.439.322%201.912.891l186.313%20223.096a31.95%2031.95%200%200%200%2024.357%2011.297h6.508c17.623%200%2031.927-14.272%2031.938-31.895V138.116c0-17.623-14.294-31.917-31.917-31.917%22%2F%3E%3C%2Fsvg%3E",
                heading: "NEAR Connector",
                link: "https://wallet.near.org",
                linkText: "Don't have a wallet?",
            };
        }
        this.whenManifestLoaded = new Promise(async (resolve) => {
            if (options?.manifest == null || typeof options.manifest === "string") {
                this.manifest = await this._loadManifest(options?.manifest).catch(() => ({ wallets: [], version: "1.0.0" }));
            }
            else {
                this.manifest = options?.manifest ?? { wallets: [], version: "1.0.0" };
            }
            const set = new Set(this.excludedWallets);
            set.delete("hot-wallet"); // always include hot-wallet
            this.manifest.wallets = this.manifest.wallets.filter((wallet) => {
                // Remove wallet with walletConnect permission but no projectId is provided
                if (wallet.permissions.walletConnect && !this.walletConnect)
                    return false;
                if (set.has(wallet.id))
                    return false; // excluded wallets
                return true;
            });
            await new Promise((resolve) => setTimeout(resolve, 100));
            resolve();
        });
        if (typeof window !== "undefined") {
            window.addEventListener("near-wallet-injected", this._handleNearWalletInjected);
            window.dispatchEvent(new Event("near-selector-ready"));
            window.addEventListener("message", async (event) => {
                if (event.data.type === "near-wallet-injected") {
                    await this.whenManifestLoaded.catch(() => { });
                    this.wallets = this.wallets.filter((wallet) => wallet.manifest.id !== event.data.manifest.id);
                    this.wallets.unshift(new ParentFrameWallet_1.ParentFrameWallet(this, event.data.manifest));
                    this.events.emit("selector:walletsChanged", {});
                    if (this.autoConnect) {
                        this.connect({ walletId: event.data.manifest.id });
                    }
                }
            });
        }
        this.whenManifestLoaded.then(() => {
            if (typeof window !== "undefined") {
                window.parent.postMessage({ type: "near-selector-ready" }, "*");
            }
            this.manifest.wallets.forEach((wallet) => this.registerWallet(wallet));
            this.storage.get("debug-wallets").then((json) => {
                const debugWallets = JSON.parse(json ?? "[]");
                debugWallets.forEach((wallet) => this.registerDebugWallet(wallet));
            });
        });
    }
    get availableWallets() {
        const wallets = this.wallets.filter((wallet) => {
            return Object.entries(this.features).every(([key, value]) => {
                if (value && !wallet.manifest.features?.[key])
                    return false;
                return true;
            });
        });
        return wallets.filter((wallet) => {
            if (this.network === "testnet" && !wallet.manifest.features?.testnet)
                return false;
            return true;
        });
    }
    _handleNearWalletInjected = (event) => {
        this.wallets = this.wallets.filter((wallet) => wallet.manifest.id !== event.detail.manifest.id);
        this.wallets.unshift(new InjectedWallet_1.InjectedWallet(this, event.detail));
        this.events.emit("selector:walletsChanged", {});
    };
    async _loadManifest(manifestUrl) {
        const manifestEndpoints = manifestUrl ? [manifestUrl] : defaultManifests;
        for (const endpoint of manifestEndpoints) {
            const res = await fetch(endpoint).catch(() => null);
            if (!res || !res.ok)
                continue;
            return await res.json(); // TODO: Validate this
        }
        throw new Error("Failed to load manifest");
    }
    async switchNetwork(network, connectOptions) {
        if (this.network === network)
            return;
        await this.disconnect().catch(() => { });
        this.network = network;
        await this.connect(connectOptions);
    }
    async registerWallet(manifest) {
        if (manifest.type !== "sandbox")
            throw new Error("Only sandbox wallets are supported");
        if (this.wallets.find((wallet) => wallet.manifest.id === manifest.id))
            return;
        this.wallets.push(new SandboxedWallet_1.SandboxWallet(this, manifest));
        this.events.emit("selector:walletsChanged", {});
    }
    async registerDebugWallet(json) {
        const manifest = typeof json === "string" ? JSON.parse(json) : json;
        if (manifest.type !== "sandbox")
            throw new Error("Only sandbox wallets type are supported");
        if (!manifest.id)
            throw new Error("Manifest must have an id");
        if (!manifest.name)
            throw new Error("Manifest must have a name");
        if (!manifest.icon)
            throw new Error("Manifest must have an icon");
        if (!manifest.website)
            throw new Error("Manifest must have a website");
        if (!manifest.version)
            throw new Error("Manifest must have a version");
        if (!manifest.executor)
            throw new Error("Manifest must have an executor");
        if (!manifest.features)
            throw new Error("Manifest must have features");
        if (!manifest.permissions)
            throw new Error("Manifest must have permissions");
        if (this.wallets.find((wallet) => wallet.manifest.id === manifest.id))
            throw new Error("Wallet already registered");
        manifest.debug = true;
        this.wallets.unshift(new SandboxedWallet_1.SandboxWallet(this, manifest));
        this.events.emit("selector:walletsChanged", {});
        const debugWallets = this.wallets.filter((wallet) => wallet.manifest.debug).map((wallet) => wallet.manifest);
        this.storage.set("debug-wallets", JSON.stringify(debugWallets));
        return manifest;
    }
    async removeDebugWallet(id) {
        this.wallets = this.wallets.filter((wallet) => wallet.manifest.id !== id);
        const debugWallets = this.wallets.filter((wallet) => wallet.manifest.debug).map((wallet) => wallet.manifest);
        this.storage.set("debug-wallets", JSON.stringify(debugWallets));
        this.events.emit("selector:walletsChanged", {});
    }
    async selectWallet({ features = {} } = {}) {
        await this.whenManifestLoaded.catch(() => { });
        return new Promise((resolve, reject) => {
            const popup = new NearWalletsPopup_1.NearWalletsPopup({
                footer: this.footerBranding,
                wallets: this.availableWallets.filter(createFilterForWalletFeatures(features)).map((wallet) => wallet.manifest),
                onRemoveDebugManifest: async (id) => this.removeDebugWallet(id),
                onAddDebugManifest: async (wallet) => this.registerDebugWallet(wallet),
                onReject: () => (reject(new Error("User rejected")), popup.destroy()),
                onSelect: (id) => (resolve(id), popup.destroy()),
            });
            popup.create();
        });
    }
    async connect(input = {}) {
        let walletId = input.walletId;
        const signMessageParams = input.signMessageParams;
        await this.whenManifestLoaded.catch(() => { });
        if (!walletId) {
            walletId = await this.selectWallet({
                features: {
                    signInAndSignMessage: input.signMessageParams != null ? true : undefined,
                    signInWithFunctionCallKey: input.addFunctionCallKey != null ? true : undefined,
                },
            });
        }
        try {
            const wallet = await this.wallet(walletId);
            this.logger?.log(`Wallet available to connect`, wallet);
            await this.storage.set("selected-wallet", walletId);
            this.logger?.log(`Set preferred wallet, try to signIn${signMessageParams != null ? " (with signed message)" : ""}`, walletId);
            let addFunctionCallKey = undefined;
            if (input.addFunctionCallKey != null) {
                this.logger?.log(`Adding function call access key during sign in with params`, input.addFunctionCallKey);
                // Set default gas allowance if not provided
                addFunctionCallKey = {
                    ...input.addFunctionCallKey,
                    gasAllowance: input.addFunctionCallKey.gasAllowance ?? {
                        amount: "250000000000000000000000", // 0.25 NEAR in yoctoNEAR
                        kind: "limited",
                    },
                };
            }
            if (signMessageParams != null) {
                const accounts = await wallet.signInAndSignMessage({
                    addFunctionCallKey,
                    messageParams: signMessageParams,
                    network: this.network,
                });
                if (!accounts?.length)
                    throw new Error("Failed to sign in");
                this.logger?.log(`Signed in to wallet (with signed message)`, walletId, accounts);
                this.events.emit("wallet:signInAndSignMessage", { wallet, accounts, success: true });
                this.events.emit("wallet:signIn", {
                    wallet,
                    accounts: accounts.map((account) => ({
                        accountId: account.accountId,
                        publicKey: account.publicKey,
                    })),
                    success: true,
                    source: "signInAndSignMessage",
                });
            }
            else {
                const accounts = await wallet.signIn({
                    addFunctionCallKey,
                    network: this.network,
                });
                if (!accounts?.length)
                    throw new Error("Failed to sign in");
                this.logger?.log(`Signed in to wallet`, walletId, accounts);
                this.events.emit("wallet:signIn", { wallet, accounts, success: true, source: "signIn" });
            }
            return wallet;
        }
        catch (e) {
            this.logger?.log("Failed to connect to wallet", e);
            throw e;
        }
    }
    async disconnect(wallet) {
        if (!wallet)
            wallet = await this.wallet();
        await wallet.signOut({ network: this.network });
        await this.storage.remove("selected-wallet");
        this.events.emit("wallet:signOut", { success: true });
    }
    async getConnectedWallet() {
        await this.whenManifestLoaded.catch(() => { });
        const id = await this.storage.get("selected-wallet");
        const wallet = this.wallets.find((wallet) => wallet.manifest.id === id);
        if (!wallet)
            throw new Error("No wallet selected");
        const accounts = await wallet.getAccounts();
        if (!accounts?.length)
            throw new Error("No accounts found");
        return { wallet, accounts };
    }
    async wallet(id) {
        await this.whenManifestLoaded.catch(() => { });
        if (!id) {
            return this.getConnectedWallet()
                .then(({ wallet }) => wallet)
                .catch(async () => {
                await this.storage.remove("selected-wallet");
                throw new Error("No accounts found");
            });
        }
        const wallet = this.wallets.find((wallet) => wallet.manifest.id === id);
        if (!wallet)
            throw new Error("Wallet not found");
        return wallet;
    }
    async use(plugin) {
        await this.whenManifestLoaded.catch(() => { });
        this.wallets = this.wallets.map((wallet) => {
            return new Proxy(wallet, {
                get(target, prop, receiver) {
                    const originalValue = Reflect.get(target, prop, receiver);
                    // If plugin has this method and it's a function on the wallet
                    if (prop in plugin && typeof originalValue === "function") {
                        const pluginMethod = plugin[prop];
                        // Act as middleware, can call next method in line via next()
                        return function (...args) {
                            const next = () => originalValue.apply(target, args);
                            // Pass all args if any exist, otherwise undefined
                            // this ensures next is always the last param
                            return args.length > 0 ? pluginMethod.call(this, ...args, next) : pluginMethod.call(this, undefined, next);
                        };
                    }
                    return originalValue;
                },
            });
        });
    }
    on(event, callback) {
        this.events.on(event, callback);
    }
    once(event, callback) {
        this.events.once(event, callback);
    }
    off(event, callback) {
        this.events.off(event, callback);
    }
    removeAllListeners(event) {
        this.events.removeAllListeners(event);
    }
}
exports.NearConnector = NearConnector;
};
defs["ParentFrameWallet.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.ParentFrameWallet = void 0;
const actions_1 = require("./actions");
const gas_keys_1 = require("./actions/gas-keys");
const uuid_1 = require("./helpers/uuid");
class ParentFrameWallet {
    connector;
    manifest;
    constructor(connector, manifest) {
        this.connector = connector;
        this.manifest = manifest;
    }
    callParentFrame(method, params) {
        const id = (0, uuid_1.uuid4)();
        window.parent.postMessage({ type: "near-wallet-injected-request", id, method, params }, "*");
        return new Promise((resolve, reject) => {
            const handler = (event) => {
                if (event.data.type === "near-wallet-injected-response" && event.data.id === id) {
                    window.removeEventListener("message", handler);
                    if (event.data.success)
                        resolve(event.data.result);
                    else
                        reject(event.data.error);
                }
            };
            window.addEventListener("message", handler);
        });
    }
    async signIn(data) {
        const result = await this.callParentFrame("near:signIn", {
            network: data?.network ?? this.connector.network,
            addFunctionCallKey: data?.addFunctionCallKey,
        });
        if (Array.isArray(result))
            return result;
        return [result];
    }
    async signInAndSignMessage(data) {
        const result = await this.callParentFrame("near:signInAndSignMessage", {
            network: data?.network ?? this.connector.network,
            addFunctionCallKey: data?.addFunctionCallKey,
            messageParams: data.messageParams,
        });
        if (Array.isArray(result))
            return result;
        return [result];
    }
    async signOut(data) {
        const args = { ...data, network: data?.network ?? this.connector.network };
        await this.callParentFrame("near:signOut", args);
    }
    async getAccounts(data) {
        const args = { ...data, network: data?.network ?? this.connector.network };
        return this.callParentFrame("near:getAccounts", args);
    }
    async signAndSendTransaction(params) {
        const connectorActions = (0, actions_1.nearActionsToConnectorActions)(params.actions);
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, connectorActions, this.manifest.name);
        const args = { ...params, actions: connectorActions, network: params.network ?? this.connector.network };
        return this.callParentFrame("near:signAndSendTransaction", args);
    }
    async signAndSendTransactions(params) {
        const transactions = params.transactions.map((transaction) => ({
            actions: (0, actions_1.nearActionsToConnectorActions)(transaction.actions),
            receiverId: transaction.receiverId,
        }));
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, transactions.flatMap((transaction) => transaction.actions), this.manifest.name);
        const args = { ...params, transactions, network: params.network ?? this.connector.network };
        return this.callParentFrame("near:signAndSendTransactions", args);
    }
    async signMessage(params) {
        const args = { ...params, network: params.network ?? this.connector.network };
        return this.callParentFrame("near:signMessage", args);
    }
    async signDelegateActions(params) {
        const args = {
            ...params,
            delegateActions: params.delegateActions.map((delegateAction) => ({
                ...delegateAction,
                actions: (0, actions_1.nearActionsToConnectorActions)(delegateAction.actions),
            })),
            network: params.network || this.connector.network,
        };
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, args.delegateActions.flatMap((delegateAction) => delegateAction.actions), this.manifest.name);
        return this.callParentFrame("near:signDelegateActions", args);
    }
}
exports.ParentFrameWallet = ParentFrameWallet;
};
defs["SandboxedWallet/code.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const nearConnectStatic_1 = require("../nearConnectStatic");
async function getIframeCode(args) {
    const storage = await args.executor.getAllStorage();
    const providers = args.executor.connector.providers;
    const manifest = args.executor.manifest;
    const uuid = args.id;
    const code = args.code
        .replaceAll(".localStorage", ".sandboxedLocalStorage")
        .replaceAll("window.top", "window.selector")
        .replaceAll("window.open", "window.selector.open");
    const nonceAttr = args.cspNonce ? ` nonce="${args.cspNonce.replace(/[^A-Za-z0-9+/=]/g, "")}"` : "";
    return /* html */ `
  <!DOCTYPE html>
  <html>
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
    </head>
    <body>
      <div id="root"></div>

      <style>
        :root {
          --background-color: rgb(40, 40, 40);
          --text-color: rgb(255, 255, 255);
          --border-color: rgb(209, 209, 209);
        }

        * {
          font-family: system-ui, Avenir, Helvetica, Arial, sans-serif
        }

        body, html {
          box-sizing: border-box;
          margin: 0;
          padding: 0;
          background-color: var(--background-color);
          color: var(--text-color);
        }

        #root {
          display: none;
          flex-direction: column;
          align-items: center;
          justify-content: center;
          height: 100vh;
          width: 100vw;
          background: radial-gradient(circle at center, #2c2c2c 0%, #1a1a1a 100%);
          text-align: center;
        }

        #root * {
          box-sizing: border-box;
          font-family: Inter, system-ui, Avenir, Helvetica, Arial, sans-serif;
          line-height: 1.5;
          color-scheme: light dark;
          color: rgb(255, 255, 255);
          font-synthesis: none;
          text-rendering: optimizeLegibility;
          -webkit-font-smoothing: antialiased;
        }

        .prompt-container img {
          width: 100px;
          height: 100px;
          object-fit: cover;
          border-radius: 12px;
        }

        .prompt-container h1 {
          margin: 0;
          font-size: 24px;
          font-weight: 600;
          margin-top: 16px;
        }

        .prompt-container p {
          margin: 0;
          font-size: 16px;
          font-weight: 500;
          color: rgb(209, 209, 209);
        }

        .prompt-container button {
          background-color: #131313;
          border: none;
          border-radius: 12px;
          padding: 12px 24px;
          cursor: pointer;
          transition: border-color 0.25s;
          color: #fff;
          outline: none;
          font-size: 14px;
          font-weight: 500;
          font-family: inherit;
          margin-top: 16px;
        }
      </style>


      <script${nonceAttr}>
      window.sandboxedLocalStorage = (() => {
        let storage = ${JSON.stringify(storage)}

        return {
          setItem: function(key, value) {
            window.selector.storage.set(key, value)
            storage[key] = value || '';
          },
          getItem: function(key) {
            return key in storage ? storage[key] : null;
          },
          removeItem: function(key) {
            window.selector.storage.remove(key)
            delete storage[key];
          },
          get length() {
            return Object.keys(storage).length;
          },
          key: function(i) {
            const keys = Object.keys(storage);
            return keys[i] || null;
          },
        };
      })();

      const showPrompt = async (args) => {
        const root = document.getElementById("root");   
        root.style.display = "flex";
        root.innerHTML = \`
          <div class="prompt-container">
            <img src="${manifest.icon}" />
            <h1>${manifest.name}</h1>
            <p>\${args.title}</p>
            <button>\${args.button}</button>
          </div>
        \`;

        return new Promise((resolve) => {
          root.querySelector("button")?.addEventListener("click", () => {
            root.innerHTML = "";
            resolve(true);
          });
        });
      }

      class ProxyWindow {
        constructor(url, features) {
          this.closed = false;
          this.windowIdPromise = window.selector.call("open", { url, features });

          window.addEventListener("message", async (event) => {            
            if (event.data.origin !== "${uuid}") return;
            if (!event.data.method?.startsWith("proxy-window:")) return;
            const method = event.data.method.replace("proxy-window:", "");
            if (method === "closed" && event.data.windowId === await this.id()) this.closed = true;
          });
        } 

        async id() {
          return await this.windowIdPromise;
        }

        async focus() {
          await window.selector.call("panel.focus", { windowId: await this.id() });
        }

        async postMessage(data) {
          window.selector.call("panel.postMessage", { windowId: await this.id(), data });
        }

        async close() {
          await window.selector.call("panel.close", { windowId: await this.id() });
        }
      }

      window.selector = {
        wallet: null,
        location: "${window.location.href}",
        nearConnectVersion: "${nearConnectStatic_1.NEAR_CONNECT_VERSION}",
        
        outerHeight: ${window.outerHeight},
        screenY: ${window.screenY},
        outerWidth: ${window.outerWidth},
        screenX: ${window.screenX},

        providers: {
          mainnet: ${JSON.stringify(providers.mainnet)},
          testnet: ${JSON.stringify(providers.testnet)},
        },

        uuid() {
          return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
            const r = (Math.random() * 16) | 0;
            const v = c === "x" ? r : (r & 0x3) | 0x8;
            return v.toString(16);
          });
        },

        walletConnect: {
          connect(params) {
            return window.selector.call("walletConnect.connect", params);
          },
          disconnect(params) {
            return window.selector.call("walletConnect.disconnect", params);
          },
          request(params) {
            return window.selector.call("walletConnect.request", params);
          },
          getProjectId() {
            return window.selector.call("walletConnect.getProjectId", {});
          },
          getSession() {
            return window.selector.call("walletConnect.getSession", {});
          },
        },
      
        async ready(wallet) {
          wallet.manifest = ${JSON.stringify(manifest)};
          window.parent.postMessage({ method: "wallet-ready", origin: "${uuid}" }, "*");
          window.selector.wallet = wallet;
        },

        async call(method, params) {
          const id = window.selector.uuid();
          window.parent.postMessage({ method, params, id, origin: "${uuid}" }, "*");

          return new Promise((resolve, reject) => {
            const handler = (event) => {
              if (event.data.id !== id || event.data.origin !== "${uuid}") return;
              window.removeEventListener("message", handler);

              if (event.data.status === "failed") reject(event.data.result);
              else resolve(event.data.result);
            };

            window.addEventListener("message", handler);
          });
        },

        panelClosed(windowId) {
          window.parent.postMessage({ 
            method: "panel.closed", 
            origin: "${uuid}", 
            result: { windowId } 
          }, "*");
        },

        open(url, _, params) {
          return new ProxyWindow(url, params)
        },

        external(entity, key, ...args) {
          return window.selector.call("external", { entity, key, args: args || [] });
        },

        openNativeApp(url) {
          return window.selector.call("open.nativeApp", { url });
        },

        ui: {
          async whenApprove(options) {
            window.selector.ui.showIframe();
            await showPrompt(options);
            window.selector.ui.hideIframe();
          },

          async showIframe() {
            return await window.selector.call("ui.showIframe");
          },

          async hideIframe() {
            return await window.selector.call("ui.hideIframe");
          },
        },

        storage: {
          async set(key, value) {
            await window.selector.call("storage.set", { key, value });
          },
      
          async get(key) {
            return await window.selector.call("storage.get", { key });
          },
      
          async remove(key) {
            await window.selector.call("storage.remove", { key });
          },

          async keys() {
            return await window.selector.call("storage.keys", {});
          },
        },
      };

      // Clipboard API often rejects inside the sandbox (Android WebView), copy through the dApp page then.
      // Plain assignment to navigator.clipboard is a no-op (getter on the prototype), so redefine the method
      const clipboard = navigator.clipboard || {};
      const nativeWriteText = clipboard.writeText ? clipboard.writeText.bind(clipboard) : null;
      Object.defineProperty(clipboard, "writeText", {
        configurable: true,
        value: async (text) => {
          try {
            if (nativeWriteText) return await nativeWriteText(text);
          } catch {}
          await window.selector.call("clipboard.writeText", { text });
        },
      });
      if (!navigator.clipboard) Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });

      window.addEventListener("message", async (event) => {
        if (event.data.origin !== "${uuid}") return;
        if (!event.data.method?.startsWith("wallet:")) return;
      
        const wallet = window.selector.wallet;
        const method = event.data.method.replace("wallet:", "");
        const payload = { id: event.data.id, origin: "${uuid}", method };
      
        if (wallet == null || typeof wallet[method] !== "function") {
          const data = { ...payload, status: "failed", result: "Method not found" };
          window.parent.postMessage(data, "*");
          return;
        }
        
        try {
          const result = await wallet[method](event.data.params);
          window.parent.postMessage({ ...payload, status: "success", result }, "*");
        } catch (error) {
          const data = { ...payload, status: "failed", result: error };
          window.parent.postMessage(data, "*");
        }
      });
      </script>

      <script type="module"${nonceAttr}>${code}</script>
    </body>
  </html>
    `;
}
exports.default = getIframeCode;
};
defs["SandboxedWallet/executor.js"] = function (module, exports, require) {
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const url_1 = require("../helpers/url");
const uuid_1 = require("../helpers/uuid");
const clipboard_1 = require("../helpers/clipboard");
const telegram_1 = require("../helpers/telegram");
const iframe_1 = __importDefault(require("./iframe"));
const cacheId = (0, uuid_1.uuid4)();
class SandboxExecutor {
    connector;
    manifest;
    activePanels = {};
    storageSpace;
    constructor(connector, manifest) {
        this.connector = connector;
        this.manifest = manifest;
        this.storageSpace = manifest.id;
    }
    checkPermissions(action, params) {
        if (action === "walletConnect") {
            return !!this.manifest.permissions.walletConnect;
        }
        if (action === "external") {
            const external = this.manifest.permissions.external;
            if (!external || !params?.entity)
                return false;
            return external.includes(params.entity);
        }
        if (action === "allowsOpen") {
            const openUrl = (0, url_1.parseUrl)(params?.url || "");
            const allowsOpen = this.manifest.permissions.allowsOpen;
            if (!openUrl || !allowsOpen || !Array.isArray(allowsOpen) || allowsOpen.length === 0)
                return false;
            const isAllowed = allowsOpen.some((path) => {
                const url = (0, url_1.parseUrl)(path);
                if (!url)
                    return false;
                if (openUrl.protocol !== url.protocol)
                    return false;
                if (!!url.hostname && openUrl.hostname !== url.hostname)
                    return false;
                if (!!url.pathname && url.pathname !== "/" && openUrl.pathname !== url.pathname)
                    return false;
                return true;
            });
            return isAllowed;
        }
        return this.manifest.permissions[action];
    }
    assertPermissions(iframe, action, event) {
        if (!this.checkPermissions(action, event.data.params)) {
            iframe.postMessage({ ...event.data, status: "failed", result: "Permission denied" });
            throw new Error("Permission denied");
        }
    }
    _onMessage = async (iframe, event) => {
        const success = (result) => {
            iframe.postMessage({ ...event.data, status: "success", result: result });
        };
        const failed = (error) => {
            iframe.postMessage({ ...event.data, status: "failed", result: error });
        };
        if (event.data.method === "ui.showIframe") {
            iframe.show();
            success(null);
            return;
        }
        if (event.data.method === "ui.hideIframe") {
            iframe.hide();
            success(null);
            return;
        }
        if (event.data.method === "storage.set") {
            this.assertPermissions(iframe, "storage", event);
            localStorage.setItem(`${this.storageSpace}:${event.data.params.key}`, event.data.params.value);
            success(null);
            return;
        }
        if (event.data.method === "storage.get") {
            this.assertPermissions(iframe, "storage", event);
            const value = localStorage.getItem(`${this.storageSpace}:${event.data.params.key}`);
            success(value);
            return;
        }
        if (event.data.method === "storage.keys") {
            this.assertPermissions(iframe, "storage", event);
            const keys = Object.keys(localStorage).filter((key) => key.startsWith(`${this.storageSpace}:`));
            success(keys);
            return;
        }
        if (event.data.method === "storage.remove") {
            this.assertPermissions(iframe, "storage", event);
            localStorage.removeItem(`${this.storageSpace}:${event.data.params.key}`);
            success(null);
            return;
        }
        if (event.data.method === "panel.focus") {
            const panel = this.activePanels[event.data.params.windowId];
            if (panel)
                panel.focus();
            success(null);
            return;
        }
        if (event.data.method === "panel.postMessage") {
            const panel = this.activePanels[event.data.params.windowId];
            if (panel)
                panel.postMessage(event.data.params.data, "*");
            success(null);
            return;
        }
        if (event.data.method === "panel.close") {
            const panel = this.activePanels[event.data.params.windowId];
            if (panel)
                panel.close();
            delete this.activePanels[event.data.params.windowId];
            success(null);
            return;
        }
        if (event.data.method === "walletConnect.connect") {
            this.assertPermissions(iframe, "walletConnect", event);
            try {
                if (!this.connector.walletConnect)
                    throw new Error("WalletConnect is not configured");
                const client = await this.connector.walletConnect;
                const result = await client.connect(event.data.params);
                result.approval();
                success({ uri: result.uri });
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "walletConnect.getProjectId") {
            if (!this.connector.walletConnect)
                throw new Error("WalletConnect is not configured");
            this.assertPermissions(iframe, "walletConnect", event);
            const client = await this.connector.walletConnect;
            success(client.core.projectId);
            return;
        }
        if (event.data.method === "walletConnect.disconnect") {
            this.assertPermissions(iframe, "walletConnect", event);
            try {
                if (!this.connector.walletConnect)
                    throw new Error("WalletConnect is not configured");
                const client = await this.connector.walletConnect;
                const result = await client.disconnect(event.data.params);
                success(result);
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "walletConnect.getSession") {
            this.assertPermissions(iframe, "walletConnect", event);
            try {
                if (!this.connector.walletConnect)
                    throw new Error("WalletConnect is not configured");
                const client = await this.connector.walletConnect;
                const key = client.session.keys[client.session.keys.length - 1];
                const session = key ? client.session.get(key) : null;
                success(session ? { topic: session.topic, namespaces: session.namespaces } : null);
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "walletConnect.request") {
            this.assertPermissions(iframe, "walletConnect", event);
            try {
                if (!this.connector.walletConnect)
                    throw new Error("WalletConnect is not configured");
                const client = await this.connector.walletConnect;
                const result = await client.request(event.data.params);
                success(result);
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "clipboard.writeText") {
            this.assertPermissions(iframe, "clipboardWrite", event);
            try {
                await (0, clipboard_1.copyText)(event.data.params.text);
                success(null);
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "external") {
            this.assertPermissions(iframe, "external", event);
            try {
                const { entity, key, args } = event.data.params;
                const obj = entity.split(".").reduce((acc, key) => acc[key], window);
                // TODO: remove hack after Nightly fixes.
                // Their method wait near.Transaction and call encode() to get bytes.
                // External API should not require non-serializable data, this is unsafe from an isolation point of view.
                if (entity === "nightly.near" && key === "signTransaction") {
                    args[0].encode = () => args[0];
                }
                const result = typeof obj[key] === "function" ? await obj[key](...(args || [])) : obj[key];
                success(result);
            }
            catch (e) {
                failed(e);
            }
            return;
        }
        if (event.data.method === "open") {
            this.assertPermissions(iframe, "allowsOpen", event);
            // Telegram Mini App blocks window.open, links go through Telegram (no panel to track)
            const tgUrl = (0, url_1.parseUrl)(event.data.params.url);
            if (tgUrl && tgUrl.protocol === "https:" && (0, telegram_1.isTelegramMiniApp)()) {
                if (tgUrl.hostname === "t.me")
                    (0, telegram_1.openTelegramLink)(tgUrl);
                else
                    (0, telegram_1.openExternalLink)(tgUrl);
                success(null);
                return;
            }
            const panel = window.open(event.data.params.url, "_blank", event.data.params.features);
            const panelId = panel ? (0, uuid_1.uuid4)() : null;
            const handler = (ev) => {
                const url = (0, url_1.parseUrl)(event.data.params.url);
                if (url && url.origin === ev.origin) {
                    iframe.postMessage(ev.data);
                }
            };
            success(panelId);
            window.addEventListener("message", handler);
            if (panel && panelId) {
                this.activePanels[panelId] = panel;
                const interval = setInterval(() => {
                    if (!panel?.closed)
                        return;
                    window.removeEventListener("message", handler);
                    const args = { method: "proxy-window:closed", windowId: panelId };
                    delete this.activePanels[panelId];
                    clearInterval(interval);
                    try {
                        iframe.postMessage(args);
                    }
                    catch { }
                }, 500);
            }
            return;
        }
        if (event.data.method === "open.nativeApp") {
            this.assertPermissions(iframe, "allowsOpen", event);
            const url = (0, url_1.parseUrl)(event.data.params.url);
            const invalid = ["https", "http", "javascript:", "file:", "data:", "blob:", "about:"];
            if (!url || invalid.includes(url.protocol)) {
                failed("Invalid URL");
                throw new Error("[open.nativeApp] Invalid URL");
            }
            const linkIframe = document.createElement("iframe");
            linkIframe.src = event.data.params.url;
            linkIframe.style.display = "none";
            document.body.appendChild(linkIframe);
            iframe.postMessage({ ...event.data, status: "success", result: null });
            return;
        }
    };
    actualCode = null;
    async checkNewVersion(executor, currentVersion) {
        if (this.actualCode) {
            this.connector.logger?.log(`New version of code already checked`);
            return this.actualCode;
        }
        let url = (0, url_1.parseUrl)(executor.manifest.executor);
        if (!url)
            url = (0, url_1.parseUrl)(location.origin + executor.manifest.executor); // relative url
        if (!url)
            throw new Error("Invalid executor URL");
        url.searchParams.set("nonce", cacheId);
        const newVersion = await fetch(url.toString()).then((res) => {
            // An error page (raw.githubusercontent 429/5xx) must not be cached and run as the executor
            if (!res.ok)
                throw new Error(`Failed to load executor: ${res.status}`);
            return res.text();
        });
        this.connector.logger?.log(`New version of code fetched`);
        this.actualCode = newVersion;
        if (newVersion === currentVersion) {
            this.connector.logger?.log(`New version of code is the same as the current version`);
            return this.actualCode;
        }
        // The cache is optional: IndexedDB fails on iOS Safari ("Connection to Indexed Database server lost", private mode),
        // and a failed write must not keep the wallet from opening
        await this.connector.db
            .setItem(`${this.manifest.id}:${this.manifest.version}`, newVersion)
            .then(() => this.connector.logger?.log(`New version of code saved to cache`))
            .catch((e) => this.connector.logger?.log(`Failed to cache new version of code`, e));
        return newVersion;
    }
    async loadCode() {
        const cachedCode = await this.connector.db.getItem(`${this.manifest.id}:${this.manifest.version}`).catch(() => null);
        this.connector.logger?.log(`Code loaded from cache`, cachedCode !== null);
        const task = this.checkNewVersion(this, cachedCode);
        if (cachedCode)
            return cachedCode;
        return await task;
    }
    async call(method, params) {
        this.connector.logger?.log(`Add to queue`, method, params);
        // return this.queue.enqueue(async () => {
        this.connector.logger?.log(`Calling method`, method, params);
        const code = await this.loadCode();
        this.connector.logger?.log(`Code loaded, preparing`);
        const iframe = new iframe_1.default(this, code, this._onMessage, this.connector.cspNonce);
        this.connector.logger?.log(`Code loaded, iframe initialized`);
        await iframe.readyPromise;
        this.connector.logger?.log(`Iframe ready`);
        const id = (0, uuid_1.uuid4)();
        return new Promise((resolve, reject) => {
            try {
                const handler = (event) => {
                    if (event.data.id !== id || event.data.origin !== iframe.origin)
                        return;
                    iframe.dispose();
                    window.removeEventListener("message", handler);
                    this.connector.logger?.log("postMessage", { result: event.data, request: { method, params } });
                    if (event.data.status === "failed")
                        reject(event.data.result);
                    else
                        resolve(event.data.result);
                };
                window.addEventListener("message", handler);
                iframe.postMessage({ method, params, id });
                iframe.on("close", () => reject(new Error("Wallet closed")));
            }
            catch (e) {
                this.connector.logger?.log(`Iframe error`, e);
                reject(e);
            }
        });
        // });
    }
    async getAllStorage() {
        const keys = Object.keys(localStorage).filter((key) => key.startsWith(`${this.storageSpace}:`));
        const storage = {};
        for (const key of keys) {
            storage[key.replace(`${this.storageSpace}:`, "")] = localStorage.getItem(key);
        }
        return storage;
    }
    async clearStorage() {
        const keys = Object.keys(localStorage).filter((key) => key.startsWith(`${this.storageSpace}:`));
        for (const key of keys) {
            localStorage.removeItem(key);
        }
    }
}
exports.default = SandboxExecutor;
};
defs["SandboxedWallet/iframe.js"] = function (module, exports, require) {
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const events_1 = require("../helpers/events");
const uuid_1 = require("../helpers/uuid");
const IframeWalletPopup_1 = require("../popups/IframeWalletPopup");
const code_1 = __importDefault(require("./code"));
class IframeExecutor {
    executor;
    origin;
    disposed = false;
    iframe = document.createElement("iframe");
    events = new events_1.EventEmitter();
    popup;
    handler;
    readyPromiseResolve;
    readyPromise = new Promise((resolve) => {
        this.readyPromiseResolve = resolve;
    });
    constructor(executor, code, onMessage, cspNonce) {
        this.executor = executor;
        this.origin = (0, uuid_1.uuid4)();
        this.handler = (event) => {
            if (event.data.origin !== this.origin)
                return;
            if (event.data.method === "wallet-ready")
                this.readyPromiseResolve();
            onMessage(this, event);
        };
        window.addEventListener("message", this.handler);
        const iframeAllowedPermissions = [];
        if (this.executor.checkPermissions("usb"))
            iframeAllowedPermissions.push("usb *;");
        if (this.executor.checkPermissions("hid"))
            iframeAllowedPermissions.push("hid *;");
        if (this.executor.checkPermissions("clipboardRead"))
            iframeAllowedPermissions.push("clipboard-read;");
        if (this.executor.checkPermissions("clipboardWrite"))
            iframeAllowedPermissions.push("clipboard-write;");
        if (this.executor.checkPermissions("bluetooth"))
            iframeAllowedPermissions.push("bluetooth *;");
        this.iframe.allow = iframeAllowedPermissions.join(" ");
        this.iframe.setAttribute("sandbox", "allow-scripts");
        (0, code_1.default)({ id: this.origin, executor: this.executor, code, cspNonce }).then((code) => {
            this.executor.connector.logger?.log(`Iframe code injected`);
            this.iframe.srcdoc = code;
        });
        this.popup = new IframeWalletPopup_1.IframeWalletPopup({
            footer: this.executor.connector.footerBranding,
            iframe: this.iframe,
            onApprove: () => { },
            onReject: () => {
                this.disposed = true;
                window.removeEventListener("message", this.handler);
                this.events.emit("close", {});
                this.popup.destroy();
            },
        });
        this.popup.create();
    }
    on(event, callback) {
        this.events.on(event, callback);
    }
    show() {
        this.popup.show();
    }
    hide() {
        this.popup.hide();
    }
    postMessage(data) {
        if (this.disposed)
            return;
        const contentWindow = this.iframe.contentWindow;
        if (!contentWindow)
            throw new Error(`Iframe not loaded for ${data?.method ?? "unknown method"}`);
        contentWindow.postMessage({ ...data, origin: this.origin }, "*");
    }
    dispose() {
        this.disposed = true;
        window.removeEventListener("message", this.handler);
        this.popup.destroy();
    }
}
exports.default = IframeExecutor;
};
defs["SandboxedWallet/index.js"] = function (module, exports, require) {
"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.SandboxWallet = void 0;
const actions_1 = require("../actions");
const gas_keys_1 = require("../actions/gas-keys");
const executor_1 = __importDefault(require("./executor"));
class SandboxWallet {
    connector;
    manifest;
    executor;
    constructor(connector, manifest) {
        this.connector = connector;
        this.manifest = manifest;
        this.executor = new executor_1.default(connector, manifest);
    }
    async signIn(data) {
        return this.executor.call("wallet:signIn", {
            network: data?.network ?? this.connector.network,
            addFunctionCallKey: data?.addFunctionCallKey,
        });
    }
    async signInAndSignMessage(data) {
        return this.executor.call("wallet:signInAndSignMessage", {
            network: data?.network ?? this.connector.network,
            addFunctionCallKey: data?.addFunctionCallKey,
            messageParams: data.messageParams,
        });
    }
    async signOut(data) {
        const args = { ...data, network: data?.network ?? this.connector.network };
        await this.executor.call("wallet:signOut", args);
        await this.executor.clearStorage();
    }
    async getAccounts(data) {
        const args = { ...data, network: data?.network ?? this.connector.network };
        return this.executor.call("wallet:getAccounts", args);
    }
    async signAndSendTransaction(params) {
        const actions = (0, actions_1.nearActionsToConnectorActions)(params.actions);
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, actions, this.manifest.name);
        const args = { ...params, actions, network: params.network ?? this.connector.network };
        return this.executor.call("wallet:signAndSendTransaction", args);
    }
    async signAndSendTransactions(params) {
        const transactions = params.transactions.map((transaction) => ({
            actions: (0, actions_1.nearActionsToConnectorActions)(transaction.actions),
            receiverId: transaction.receiverId,
        }));
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, transactions.flatMap((transaction) => transaction.actions), this.manifest.name);
        const args = { ...params, transactions, network: params.network ?? this.connector.network };
        return this.executor.call("wallet:signAndSendTransactions", args);
    }
    async signMessage(params) {
        const args = { ...params, network: params.network ?? this.connector.network };
        return this.executor.call("wallet:signMessage", args);
    }
    async signDelegateActions(params) {
        const args = {
            ...params,
            delegateActions: params.delegateActions.map((delegateAction) => ({
                ...delegateAction,
                actions: (0, actions_1.nearActionsToConnectorActions)(delegateAction.actions),
            })),
            network: params.network ?? this.connector.network,
        };
        (0, gas_keys_1.assertGasKeyActionsSupported)(this.manifest.features, args.delegateActions.flatMap((delegateAction) => delegateAction.actions), this.manifest.name);
        return this.executor.call("wallet:signDelegateActions", args);
    }
}
exports.SandboxWallet = SandboxWallet;
exports.default = SandboxWallet;
};
defs["actions/gas-keys.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.isGasKeyAction = void 0;
exports.assertGasKeyActionsSupported = assertGasKeyActionsSupported;
/** The three connector-action shapes that only a gas-key-aware wallet can sign. */
const isGasKeyAction = (action) => action.type === "TransferToGasKey" ||
    action.type === "WithdrawFromGasKey" ||
    (action.type === "AddKey" && action.params.gasKeyInfo != null);
exports.isGasKeyAction = isGasKeyAction;
/**
 * Refuse gas-key actions unless the wallet advertises `features.gasKeys`.
 *
 * Fail-closed on purpose: a wallet that predates gas keys would at best reject
 * the request, and at worst ignore the unknown `gasKeyInfo` field and add a
 * *plain* key with whatever permission it does understand. Flip the manifest
 * flag for a wallet only after a real sign-and-send has been verified with it.
 */
function assertGasKeyActionsSupported(features, actions, walletName) {
    if (features?.gasKeys)
        return;
    const offending = actions.filter(exports.isGasKeyAction).map((action) => action.type);
    if (offending.length === 0)
        return;
    throw new Error(`${walletName} does not advertise gas-key support (manifest features.gasKeys), so ${[...new Set(offending)].join(", ")} ` +
        "cannot be sent through it; a wallet that does not know gasKeyInfo could add a plain key instead. " +
        "Sign locally with a full-access key, or use a wallet whose manifest sets features.gasKeys.");
}
};
defs["actions/index.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.nearActionsToConnectorActions = void 0;
const base58_1 = require("../helpers/base58");
const deserializeArgs = (args) => {
    try {
        return JSON.parse(new TextDecoder().decode(args));
    }
    catch {
        return args;
    }
};
// near-api-js `Enum` instances store the chosen variant under `enum` and may
// carry the *inactive* variant key as an own `undefined` property, so `in`
// checks are only safe as a fallback for enum-less, hand-shaped objects
// (e.g. `{ AccountId: null }`). A variant key whose value is `null` — like
// `GlobalContractDeployMode({ AccountId: null })` — must still be detected.
const isFullAccessPermission = (permission) => permission.enum != null ? permission.enum === "fullAccess" : "fullAccess" in permission;
const isAccountIdDeployMode = (deployMode) => deployMode.enum != null ? deployMode.enum === "AccountId" : "AccountId" in deployMode;
const nearActionsToConnectorActions = (actions) => {
    return actions.map((action) => {
        if ("type" in action)
            return action;
        if (action.signedDelegate) {
            throw new Error("SignedDelegate actions cannot be sent through the wallet connector; wallets sign delegates via signDelegateActions");
        }
        if (action.functionCall) {
            return {
                type: "FunctionCall",
                params: {
                    methodName: action.functionCall.methodName,
                    args: deserializeArgs(action.functionCall.args),
                    gas: action.functionCall.gas.toString(),
                    deposit: action.functionCall.deposit.toString(),
                },
            };
        }
        if (action.deployGlobalContract) {
            return {
                type: "DeployGlobalContract",
                params: {
                    code: action.deployGlobalContract.code,
                    deployMode: isAccountIdDeployMode(action.deployGlobalContract.deployMode) ? "AccountId" : "CodeHash",
                },
            };
        }
        if (action.createAccount) {
            return { type: "CreateAccount" };
        }
        if (action.useGlobalContract) {
            const identifier = action.useGlobalContract.contractIdentifier;
            return {
                type: "UseGlobalContract",
                params: {
                    contractIdentifier: identifier.AccountId != null
                        ? { accountId: identifier.AccountId }
                        : { codeHash: (0, base58_1.encodeBase58)(identifier.CodeHash) },
                },
            };
        }
        if (action.deployContract) {
            return {
                type: "DeployContract",
                params: { code: action.deployContract.code },
            };
        }
        if (action.deleteAccount) {
            return {
                type: "DeleteAccount",
                params: { beneficiaryId: action.deleteAccount.beneficiaryId },
            };
        }
        if (action.deleteKey) {
            return {
                type: "DeleteKey",
                params: { publicKey: action.deleteKey.publicKey.toString() },
            };
        }
        if (action.transfer) {
            return {
                type: "Transfer",
                params: { deposit: action.transfer.deposit.toString() },
            };
        }
        if (action.stake) {
            return {
                type: "Stake",
                params: {
                    stake: action.stake.stake.toString(),
                    publicKey: action.stake.publicKey.toString(),
                },
            };
        }
        if (action.addKey) {
            const { permission } = action.addKey.accessKey;
            let mapped;
            if (permission.functionCall) {
                mapped = {
                    receiverId: permission.functionCall.receiverId,
                    allowance: permission.functionCall.allowance?.toString(),
                    methodNames: permission.functionCall.methodNames,
                };
            }
            else if (isFullAccessPermission(permission)) {
                mapped = "FullAccess";
            }
            else {
                // Never guess: an unknown permission (a typo, a gas-key permission, ...)
                // must not be escalated to a full-access key.
                throw new Error("Unsupported access-key permission on AddKey: only functionCall and fullAccess can be sent through the wallet connector (pass a ConnectorAction otherwise)");
            }
            return {
                type: "AddKey",
                params: {
                    publicKey: action.addKey.publicKey.toString(),
                    accessKey: {
                        nonce: Number(action.addKey.accessKey.nonce),
                        permission: mapped,
                    },
                },
            };
        }
        throw new Error("Unsupported action type");
    });
};
exports.nearActionsToConnectorActions = nearActionsToConnectorActions;
};
defs["actions/types.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
};
defs["helpers/base58.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.encodeBase58 = encodeBase58;
// Base58 alphabet (Bitcoin)
const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
/**
 * Encodes a Uint8Array into a base58 string.
 * @param {Uint8Array} bytes
 * @returns {string}
 */
function encodeBase58(bytes) {
    if (bytes.length === 0)
        return "";
    // Count leading zeros.
    let zeros = 0;
    let i = 0;
    while (i < bytes.length && bytes[i] === 0) {
        zeros++;
        i++;
    }
    // Convert byte array to base58 digits (big-endian).
    let digits = [0];
    for (; i < bytes.length; i++) {
        let carry = bytes[i];
        for (let j = 0; j < digits.length; ++j) {
            carry += digits[j] << 8;
            digits[j] = carry % 58;
            carry = (carry / 58) | 0;
        }
        while (carry > 0) {
            digits.push(carry % 58);
            carry = (carry / 58) | 0;
        }
    }
    // Add leading zeros in base58.
    while (digits.length > 0 && digits[digits.length - 1] === 0)
        digits.pop();
    let result = "";
    for (let k = 0; k < zeros; k++) {
        result += BASE58_ALPHABET[0];
    }
    for (let q = digits.length - 1; q >= 0; --q) {
        result += BASE58_ALPHABET[digits[q]];
    }
    return result;
}
};
defs["helpers/clipboard.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.copyText = void 0;
// Clipboard API is missing or rejects in some webviews (Android in-app browsers), execCommand still works there
const copyText = async (text) => {
    try {
        await navigator.clipboard.writeText(text);
        return;
    }
    catch { }
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    try {
        if (!document.execCommand("copy"))
            throw new Error("Copy failed");
    }
    finally {
        textarea.remove();
    }
};
exports.copyText = copyText;
};
defs["helpers/events.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.EventEmitter = void 0;
/**
 * Generic event emitter class for handling typed events and their callbacks
 * @template T Record type containing event names as keys and their payload types as values
 */
class EventEmitter {
    /** Internal storage for event callbacks */
    events = {};
    /**
     * Subscribe to an event
     * @template K Event name type
     * @param event Name of the event to subscribe to
     * @param callback Function to be called when event is emitted
     */
    on(event, callback) {
        if (!this.events[event])
            this.events[event] = [];
        this.events[event].push(callback);
    }
    /**
     * Emit an event with payload
     * @template K Event name type
     * @param event Name of the event to emit
     * @param payload Data to pass to event handlers
     */
    emit(event, payload) {
        this.events[event]?.forEach((cb) => cb(payload));
    }
    /**
     * Unsubscribe from an event
     * @template K Event name type
     * @param event Name of the event to unsubscribe from
     * @param callback Function to remove from event handlers
     */
    off(event, callback) {
        this.events[event] = this.events[event]?.filter((cb) => cb !== callback);
    }
    /**
     * Subscribe to an event for a single emission
     * @template K Event name type
     * @param event Name of the event to subscribe to
     * @param callback Function to be called when event is emitted
     */
    once(event, callback) {
        const onceWrapper = (payload) => {
            callback(payload);
            this.off(event, onceWrapper);
        };
        this.on(event, onceWrapper);
    }
    /**
     * Remove all event listeners
     * @template K Event name type
     * @param event Optional event name to remove listeners for. If not provided, removes all listeners for all events
     */
    removeAllListeners(event) {
        if (event) {
            delete this.events[event];
        }
        else {
            this.events = {};
        }
    }
}
exports.EventEmitter = EventEmitter;
};
defs["helpers/html.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.escapeHtml = escapeHtml;
exports.html = html;
function escapeHtml(unsafe) {
    return unsafe
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}
const htmlTag = Symbol("htmlTag");
function html(strings, ...values) {
    let result = strings[0];
    for (let i = 0; i < values.length; i++) {
        for (const value of Array.isArray(values[i]) ? values[i] : [values[i]]) {
            const escaped = value?.[htmlTag] ? value[htmlTag] : escapeHtml(String(value ?? ""));
            result += escaped;
        }
        result += strings[i + 1];
    }
    return Object.freeze({
        [htmlTag]: result,
        get html() {
            return result;
        },
    });
}
};
defs["helpers/indexdb.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
class IndexedDB {
    dbName;
    storeName;
    version;
    constructor(dbName, storeName) {
        this.dbName = dbName;
        this.storeName = storeName;
        this.version = 1;
    }
    getDb() {
        return new Promise((resolve, reject) => {
            if (typeof window === "undefined" || typeof indexedDB === "undefined") {
                reject(new Error("IndexedDB is not available (SSR environment)"));
                return;
            }
            const request = indexedDB.open(this.dbName, this.version);
            request.onerror = (event) => {
                console.error("Error opening database:", event.target.error);
                reject(new Error("Error opening database"));
            };
            request.onsuccess = (event) => {
                resolve(request.result);
            };
            request.onupgradeneeded = (event) => {
                const db = request.result;
                const existingStores = db.objectStoreNames;
                if (!existingStores.contains(this.storeName)) {
                    db.createObjectStore(this.storeName);
                }
            };
        });
    }
    async getItem(key) {
        const db = await this.getDb();
        if (typeof key === "number") {
            key = key.toString();
        }
        if (typeof key !== "string") {
            throw new Error("Key must be a string");
        }
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readonly");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.get(key);
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                resolve(request.result);
                db.close();
            };
        });
    }
    async setItem(key, value) {
        const db = await this.getDb();
        if (typeof key === "number") {
            key = key.toString();
        }
        if (typeof key !== "string") {
            throw new Error("Key must be a string");
        }
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readwrite");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.put(value, key);
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                db.close();
                resolve();
            };
        });
    }
    async removeItem(key) {
        const db = await this.getDb();
        if (typeof key === "number") {
            key = key.toString();
        }
        if (typeof key !== "string") {
            throw new Error("Key must be a string");
        }
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readwrite");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.delete(key);
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                db.close();
                resolve();
            };
        });
    }
    async keys() {
        const db = await this.getDb();
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readonly");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.getAllKeys();
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                resolve(request.result);
                db.close();
            };
        });
    }
    async count() {
        const db = await this.getDb();
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readonly");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.count();
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                resolve(request.result);
                db.close();
            };
        });
    }
    async length() {
        return this.count();
    }
    async clear() {
        const db = await this.getDb();
        return new Promise((resolve, reject) => {
            if (!this.storeName) {
                reject(new Error("Store name not set"));
                return;
            }
            const transaction = db.transaction(this.storeName, "readwrite");
            transaction.onerror = (event) => reject(transaction.error);
            const store = transaction.objectStore(this.storeName);
            const request = store.clear();
            request.onerror = (event) => reject(request.error);
            request.onsuccess = () => {
                db.close();
                resolve();
            };
        });
    }
}
exports.default = IndexedDB;
};
defs["helpers/queue.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.AutoQueue = exports.Queue = void 0;
class Queue {
    _items = [];
    enqueue(item) {
        this._items.push(item);
    }
    dequeue() {
        return this._items.shift();
    }
    get size() {
        return this._items.length;
    }
}
exports.Queue = Queue;
class AutoQueue extends Queue {
    _pendingPromise = false;
    enqueue(action) {
        return new Promise((resolve, reject) => {
            super.enqueue({ action, resolve, reject });
            this.dequeue();
        });
    }
    async dequeue() {
        if (this._pendingPromise)
            return false;
        const item = super.dequeue();
        if (!item)
            return false;
        try {
            this._pendingPromise = true;
            const payload = await item.action(this);
            this._pendingPromise = false;
            item.resolve(payload);
        }
        catch (e) {
            this._pendingPromise = false;
            item.reject(e);
        }
        finally {
            void this.dequeue();
        }
        return true;
    }
}
exports.AutoQueue = AutoQueue;
};
defs["helpers/storage.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.LocalStorage = void 0;
class LocalStorage {
    async get(key) {
        if (typeof window === "undefined")
            return null;
        return localStorage.getItem(key);
    }
    async set(key, value) {
        if (typeof window === "undefined")
            return;
        localStorage.setItem(key, value);
    }
    async remove(key) {
        if (typeof window === "undefined")
            return;
        localStorage.removeItem(key);
    }
}
exports.LocalStorage = LocalStorage;
};
defs["helpers/telegram.js"] = function (module, exports, require) {
"use strict";
// Telegram Mini App bridge. Inside a Mini App the webview blocks window.open
// (the call comes from a postMessage handler, not a user gesture) and never routes custom schemes,
// so links have to go through Telegram itself. The dApp may not load telegram-web-app.js,
// so we fall back to the raw bridge: https://core.telegram.org/api/web-events
Object.defineProperty(exports, "__esModule", { value: true });
exports.openExternalLink = exports.openTelegramLink = exports.isTelegramMiniApp = void 0;
const getWebApp = () => {
    const tgapp = window?.Telegram?.WebApp;
    // Outside of Telegram the SDK still exists but reports "unknown" platform
    if (!tgapp || !tgapp.platform || tgapp.platform === "unknown")
        return null;
    return tgapp;
};
const hasLaunchParams = () => {
    try {
        if (location.hash.includes("tgWebAppData"))
            return true;
        return !!sessionStorage.getItem("__telegram__initParams");
    }
    catch {
        return false;
    }
};
const isTelegramMiniApp = () => {
    if (typeof window === "undefined")
        return false;
    return !!getWebApp() || !!window.TelegramWebviewProxy || hasLaunchParams();
};
exports.isTelegramMiniApp = isTelegramMiniApp;
const postEvent = (eventType, eventData) => {
    const proxy = window.TelegramWebviewProxy;
    if (proxy?.postEvent)
        return proxy.postEvent(eventType, JSON.stringify(eventData)); // iOS, Android
    const external = window.external;
    if (external && "notify" in external)
        return external.notify(JSON.stringify({ eventType, eventData })); // Desktop
    if (window.parent !== window) {
        window.parent.postMessage(JSON.stringify({ eventType, eventData }), "https://web.telegram.org"); // Web
    }
};
const openTelegramLink = (url) => {
    const tgapp = getWebApp();
    if (tgapp)
        return tgapp.openTelegramLink(url.toString());
    postEvent("web_app_open_tg_link", { path_full: url.pathname + url.search });
};
exports.openTelegramLink = openTelegramLink;
const openExternalLink = (url) => {
    const tgapp = getWebApp();
    if (tgapp)
        return tgapp.openLink(url.toString());
    postEvent("web_app_open_link", { url: url.toString() });
};
exports.openExternalLink = openExternalLink;
};
defs["helpers/url.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.parseUrl = void 0;
const parseUrl = (url) => {
    try {
        return new URL(url);
    }
    catch {
        return null;
    }
};
exports.parseUrl = parseUrl;
};
defs["helpers/uuid.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.uuid4 = void 0;
const uuid4 = () => {
    if (typeof window !== "undefined" && typeof window.crypto !== "undefined" && typeof window.crypto.randomUUID === "function")
        return window.crypto.randomUUID();
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
        const r = (Math.random() * 16) | 0;
        const v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
};
exports.uuid4 = uuid4;
};
defs["index.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.assertGasKeyActionsSupported = exports.isGasKeyAction = exports.nearActionsToConnectorActions = exports.NearConnector = exports.InjectedWallet = exports.SandboxWallet = exports.ParentFrameWallet = exports.LocalStorage = void 0;
var storage_1 = require("./helpers/storage");
Object.defineProperty(exports, "LocalStorage", { enumerable: true, get: function () { return storage_1.LocalStorage; } });
var ParentFrameWallet_1 = require("./ParentFrameWallet");
Object.defineProperty(exports, "ParentFrameWallet", { enumerable: true, get: function () { return ParentFrameWallet_1.ParentFrameWallet; } });
var SandboxedWallet_1 = require("./SandboxedWallet");
Object.defineProperty(exports, "SandboxWallet", { enumerable: true, get: function () { return SandboxedWallet_1.SandboxWallet; } });
var InjectedWallet_1 = require("./InjectedWallet");
Object.defineProperty(exports, "InjectedWallet", { enumerable: true, get: function () { return InjectedWallet_1.InjectedWallet; } });
var NearConnector_1 = require("./NearConnector");
Object.defineProperty(exports, "NearConnector", { enumerable: true, get: function () { return NearConnector_1.NearConnector; } });
var actions_1 = require("./actions");
Object.defineProperty(exports, "nearActionsToConnectorActions", { enumerable: true, get: function () { return actions_1.nearActionsToConnectorActions; } });
var gas_keys_1 = require("./actions/gas-keys");
Object.defineProperty(exports, "isGasKeyAction", { enumerable: true, get: function () { return gas_keys_1.isGasKeyAction; } });
Object.defineProperty(exports, "assertGasKeyActionsSupported", { enumerable: true, get: function () { return gas_keys_1.assertGasKeyActionsSupported; } });
};
defs["nearConnectStatic.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.NEAR_CONNECT_VERSION = void 0;
exports.NEAR_CONNECT_VERSION = "0.12.0";
};
defs["popups/IframeWalletPopup.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.IframeWalletPopup = void 0;
const html_1 = require("../helpers/html");
const Popup_1 = require("./Popup");
class IframeWalletPopup extends Popup_1.Popup {
    delegate;
    constructor(delegate) {
        super(delegate);
        this.delegate = delegate;
    }
    handlers() {
        super.handlers();
        this.addListener("button", "click", () => this.delegate.onApprove());
    }
    create() {
        super.create({ show: false });
        const modalBody = this.root.querySelector(".modal-body");
        modalBody.appendChild(this.delegate.iframe);
        this.delegate.iframe.style.width = "100%";
        this.delegate.iframe.style.height = "720px";
        this.delegate.iframe.style.border = "none";
    }
    get footer() {
        if (!this.delegate.footer)
            return "";
        const { icon, heading } = this.delegate.footer;
        return (0, html_1.html) `
      <div class="footer">
        ${icon ? (0, html_1.html) `<img src="${icon}" alt="${heading}" />` : ""}
        <p>${heading}</p>
      </div>
    `;
    }
    get dom() {
        return (0, html_1.html) `<div class="modal-container">
      <div class="modal-content">
        <div class="modal-body" style="padding: 0; overflow: auto;"></div>
        ${this.footer}
      </div>
    </div>`;
    }
}
exports.IframeWalletPopup = IframeWalletPopup;
};
defs["popups/NearWalletsPopup.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.NearWalletsPopup = void 0;
const html_1 = require("../helpers/html");
const url_1 = require("../helpers/url");
const Popup_1 = require("./Popup");
const debugManifest = {
    id: "custom-wallet",
    name: "Custom Wallet",
    icon: "https://www.mynearwallet.com/images/webclip.png",
    description: "Custom wallet for NEAR.",
    website: "",
    version: "1.0.0",
    executor: "your-executor-url.js",
    type: "sandbox",
    platform: {},
    features: {
        signMessage: true,
        signInWithoutAddKey: true,
        signInAndSignMessage: true,
        signAndSendTransaction: true,
        signAndSendTransactions: true,
        signDelegateActions: true,
    },
    permissions: {
        storage: true,
        allowsOpen: [],
    },
};
class NearWalletsPopup extends Popup_1.Popup {
    delegate;
    constructor(delegate) {
        super(delegate);
        this.delegate = delegate;
        this.update({ wallets: delegate.wallets, showSettings: false });
    }
    handlers() {
        super.handlers();
        this.addListener(".settings-button", "click", () => this.update({ showSettings: true }));
        this.addListener(".back-button", "click", () => this.update({ showSettings: false }));
        this.root.querySelectorAll(".connect-item").forEach((item) => {
            if (!(item instanceof HTMLDivElement))
                return;
            this.addListener(item, "click", () => this.delegate.onSelect(item.dataset.type));
        });
        this.root.querySelectorAll(".remove-wallet-button").forEach((item) => {
            if (!(item instanceof SVGSVGElement))
                return;
            this.addListener(item, "click", async (e) => {
                e.stopPropagation();
                await this.delegate.onRemoveDebugManifest(item.dataset.type);
                const wallets = this.state.wallets.filter((wallet) => wallet.id !== item.dataset.type);
                this.update({ wallets });
            });
        });
        this.addListener(".add-debug-manifest-button", "click", async () => {
            try {
                const wallet = this.root.querySelector("#debug-manifest-input")?.value ?? "";
                const manifest = await this.delegate.onAddDebugManifest(wallet);
                this.update({ showSettings: false, wallets: [manifest, ...this.state.wallets] });
            }
            catch (error) {
                alert(`Something went wrong: ${error}`);
            }
        });
    }
    create() {
        super.create({ show: true });
    }
    walletDom(wallet) {
        const removeButton = (0, html_1.html) `
      <svg
        class="remove-wallet-button"
        data-type="${wallet.id}"
        width="24"
        height="24"
        viewBox="0 0 24 24"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        style="margin-right: 4px;"
      >
        <path d="M18 6L6 18" stroke="rgba(255,255,255,0.5)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M6 6L18 18" stroke="rgba(255,255,255,0.5)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    `;
        return (0, html_1.html) `
      <div class="connect-item" data-type="${wallet.id}">
        <img style="background: #333" src="${wallet.icon}" alt="${wallet.name}" />
        <div class="connect-item-info">
          <span>${wallet.name}</span>
          <span class="wallet-address">${(0, url_1.parseUrl)(wallet.website)?.hostname}</span>
        </div>
        ${wallet.debug ? removeButton : ""}
      </div>
    `;
    }
    get footer() {
        if (!this.delegate.footer)
            return "";
        const { icon, heading, link, linkText } = this.delegate.footer;
        return (0, html_1.html) `
      <div class="footer">
        ${icon ? (0, html_1.html) `<img src="${icon}" alt="${heading}" />` : ""}
        <p>${heading}</p>
        <a class="get-wallet-link" href="${link}" target="_blank">${linkText}</a>
      </div>
    `;
    }
    get dom() {
        if (this.state.showSettings) {
            return (0, html_1.html) `
        <div class="modal-container">
          <div class="modal-content">
            <div class="modal-header">
              <button class="back-button" style="left: 16px; right: unset;">
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                  <path d="M15 18L9 12L15 6" stroke="rgba(255,255,255,0.5)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
              <p>Settings</p>
            </div>

            <div class="modal-body">
              <p style="text-align: left;">
                You can add your wallet to dapp for debug,
                <a href="https://github.com/azbang/hot-connector" target="_blank">read the documentation.</a> Paste your manifest and click "Add".
              </p>

              <textarea style="width: 100%;" id="debug-manifest-input" rows="10">${JSON.stringify(debugManifest, null, 2)}</textarea>
              <button class="add-debug-manifest-button">Add</button>
            </div>

            ${this.footer}
          </div>
        </div>
      `;
        }
        return (0, html_1.html) `<div class="modal-container">
      <div class="modal-content">
        <div class="modal-header">
          <p>Select wallet</p>
          <button class="settings-button">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <circle cx="12" cy="12" r="2" fill="rgba(255,255,255,0.5)" />
              <circle cx="19" cy="12" r="2" fill="rgba(255,255,255,0.5)" />
              <circle cx="5" cy="12" r="2" fill="rgba(255,255,255,0.5)" />
            </svg>
          </button>
        </div>

        <div class="modal-body">${this.state.wallets.map((wallet) => this.walletDom(wallet))}</div>

        ${this.footer}
      </div>
    </div>`;
    }
}
exports.NearWalletsPopup = NearWalletsPopup;
};
defs["popups/Popup.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.Popup = void 0;
const styles_1 = require("./styles");
const html_1 = require("../helpers/html");
const ID = `n${Math.random().toString(36).substring(2, 15)}`;
if (typeof document !== "undefined") {
    const style = document.createElement("style");
    style.textContent = (0, styles_1.css)(`.${ID}`);
    document.head.append(style);
}
class Popup {
    delegate;
    isClosed = false;
    root = document.createElement("div");
    state = {};
    constructor(delegate) {
        this.delegate = delegate;
    }
    get dom() {
        return (0, html_1.html) ``;
    }
    disposables = [];
    addListener(querySelector, event, callback) {
        const element = typeof querySelector === "string" ? this.root.querySelector(querySelector) : querySelector;
        if (!element)
            return;
        element.addEventListener(event, callback);
        this.disposables.push(() => element.removeEventListener(event, callback));
    }
    handlers() {
        this.disposables.forEach((dispose) => dispose());
        this.disposables = [];
        const modalContainer = this.root.querySelector(".modal-container");
        const modalContent = this.root.querySelector(".modal-content");
        modalContent.onclick = (e) => e.stopPropagation();
        modalContainer.onclick = () => {
            this.delegate.onReject();
            this.destroy();
        };
        // Escape closes the popup with the same semantics as clicking the
        // backdrop. Document-level, so it works regardless of focus.
        const onKeydown = (e) => {
            if (e.key !== "Escape")
                return;
            this.delegate.onReject();
            this.destroy();
        };
        document.addEventListener("keydown", onKeydown);
        this.disposables.push(() => document.removeEventListener("keydown", onKeydown));
    }
    update(state) {
        this.state = { ...this.state, ...state };
        this.root.innerHTML = this.dom.html;
        this.handlers();
    }
    create({ show = true }) {
        this.root.className = `${ID} hot-connector-popup`;
        this.root.style.display = "none";
        this.root.innerHTML = this.dom.html;
        document.body.append(this.root);
        this.handlers();
        const modalContainer = this.root.querySelector(".modal-container");
        const modalContent = this.root.querySelector(".modal-content");
        modalContent.style.transform = "translateY(50px)";
        modalContainer.style.opacity = "0";
        if (show) {
            setTimeout(() => this.show(), 10);
        }
    }
    show() {
        const modalContainer = this.root.querySelector(".modal-container");
        const modalContent = this.root.querySelector(".modal-content");
        modalContent.style.transform = "translateY(50px)";
        modalContainer.style.opacity = "0";
        this.root.style.display = "block";
        setTimeout(() => {
            modalContent.style.transform = "translateY(0)";
            modalContainer.style.opacity = "1";
        }, 100);
    }
    hide() {
        const modalContainer = this.root.querySelector(".modal-container");
        const modalContent = this.root.querySelector(".modal-content");
        modalContent.style.transform = "translateY(50px)";
        modalContainer.style.opacity = "0";
        setTimeout(() => {
            this.root.style.display = "none";
        }, 200);
    }
    destroy() {
        if (this.isClosed)
            return;
        this.isClosed = true;
        // Element listeners die with root.remove(), but document-level ones
        // (Escape) must be disposed explicitly or they leak per popup open.
        this.disposables.forEach((dispose) => dispose());
        this.disposables = [];
        this.hide();
        setTimeout(() => {
            this.root.remove();
        }, 200);
    }
}
exports.Popup = Popup;
};
defs["popups/styles.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.css = void 0;
const css = (id) => /*css*/ `
${id} * {
  box-sizing: border-box;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif, "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol";
  -ms-overflow-style: none; 
  scrollbar-width: none; 
  color: #fff;
}

${id} *::-webkit-scrollbar { 
  display: none;
}

${id} p,
${id} h1,
${id} h2,
${id} h3,
${id} h4,
${id} h5,
${id} h6 {
  margin: 0;
}

${id} .modal-container {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: 100000000;
    background-color: rgba(0, 0, 0, 0.5);
    display: flex;
    justify-content: center;
    align-items: center;
    flex-direction: column;
    transition: opacity 0.2s ease-in-out;
}

@media (max-width: 600px) {
  ${id} .modal-container {
    justify-content: flex-end;
  }
}

${id} .modal-content {
  display: flex;
  flex-direction: column;
  align-items: center;

  max-width: 420px;
  max-height: 615px;
  width: 100%;
  border-radius: 24px;
  background: #0d0d0d;
  border: 1.5px solid rgba(255, 255, 255, 0.1);
  transition: transform 0.2s ease-in-out;
}

@media (max-width: 600px) {
  ${id} .modal-content {
    max-width: 100%;
    width: 100%;
    max-height: 80%;
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
    border: none;
    border-top: 1.5px solid rgba(255, 255, 255, 0.1);
  }
}


${id} .modal-header {
  display: flex;
  padding: 16px;
  gap: 16px;
  align-self: stretch;
  align-items: center;
  justify-content: center;
  position: relative;
}

${id} .modal-header button {
  position: absolute;
  right: 16px;
  top: 16px;
  width: 32px;
  height: 32px;
  border-radius: 12px;
  cursor: pointer;
  transition: background 0.2s ease-in-out;
  border: none;
  background: none;
  display: flex;
  align-items: center;
  justify-content: center;
}

${id} .modal-header button:hover {
  background: rgba(255, 255, 255, 0.04);
}
  
${id} .modal-header p {
  color: #fff;
  text-align: center;
  font-size: 24px;
  font-style: normal;
  font-weight: 600;
  line-height: normal;
  margin: 0;
}


${id} .modal-body {
  display: flex;
  padding: 16px;
  flex-direction: column;
  align-items: flex-start;
  text-align: center;
  gap: 8px;
  overflow: auto;

  border-radius: 24px;
  background: rgba(255, 255, 255, 0.08);
  width: 100%;
  flex: 1;
}

${id} .modal-body textarea {
  width: 100%;
  padding: 12px;
  border-radius: 12px;
  background: #0d0d0d;
  color: #fff;
  border: 1px solid rgba(255, 255, 255, 0.1);
  outline: none;
  font-size: 16px;
  transition: background 0.2s ease-in-out;
  font-family: monospace;
  font-size: 12px;
}

${id} .modal-body button {
  width: 100%;
  padding: 12px;
  border-radius: 12px;
  background: #fff;
  color: #000;
  border: none;
  cursor: pointer;
  font-size: 16px;
  transition: background 0.2s ease-in-out;
  margin-top: 16px;
}

${id} .footer {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  padding: 16px 24px;
  color: #fff;
  gap: 12px;
}

${id} .modal-body p {
  color: rgba(255, 255, 255, 0.9);
  text-align: center;
  font-size: 16px;
  font-style: normal;
  font-weight: 500;
  line-height: normal;
  letter-spacing: -0.8px;
}

${id} .footer img {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  object-fit: cover;
}

${id} .get-wallet-link {
  color: rgba(255, 255, 255, 0.5);
  text-align: center;
  font-size: 16px;
  font-style: normal;
  font-weight: 500;
  margin-left: auto;
  text-decoration: none;
  transition: color 0.2s ease-in-out;
  cursor: pointer;
}
  
${id} .get-wallet-link:hover {
  color: rgba(255, 255, 255, 1);
}


${id} .connect-item {
  display: flex;
  padding: 8px;
  align-items: center;
  gap: 12px;
  align-self: stretch;
  cursor: pointer;

  transition: background 0.2s ease-in-out;
  border-radius: 24px;
}

${id} .connect-item img {
  width: 48px;
  height: 48px;
  border-radius: 16px;
  object-fit: cover;
  flex-shrink: 0;
}

${id} .connect-item-info {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  text-align: left;
  flex: 1;
  margin-top: -2px;
}

${id} .connect-item-info .wallet-address {
  color: rgba(255, 255, 255, 0.5);
  font-size: 14px;
  font-style: normal;
  font-weight: 400;
  line-height: normal;
}

${id} .connect-item:hover {
  background: rgba(255, 255, 255, 0.04);
}

${id} .connect-item img {
  width: 48px;
  height: 48px;
  border-radius: 16px;
  object-fit: cover;
}

${id} .connect-item p {
  color: rgba(255, 255, 255, 0.9);
  text-align: center;
  font-size: 18px;
  font-style: normal;
  font-weight: 600;
  line-height: normal;
  letter-spacing: -0.36px;
  margin: 0;
}
`;
exports.css = css;
};
defs["types/index.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
};
defs["types/plugin.js"] = function (module, exports, require) {
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
};
const api = load("index.js");
export const NearConnector = api.NearConnector;
export default api;
