const CopyWebpackPlugin = require("copy-webpack-plugin");
const path = require('path');

module.exports = (env, argv) => ({
    module: {
        rules: [
            {
                test: /\.(css)$/,
                use: ['style-loader', 'css-loader'],
            },
            {
                test: /\.(png|jpe?g|gif)$/i,
                type: 'asset/resource',
                generator: {
                    filename: 'images/[name][ext]',
                },
            },
        ],
    },
    experiments: {
        syncWebAssembly: true,
    },
    entry: {
        bootstrap: "./bootstrap.js",
    },
    output: {
        path: path.resolve(__dirname, "dist"),
        filename: "bootstrap.js",
        clean: true,
    },
    mode: argv.mode || "development",
    devtool: argv.mode === "production" ? false : "eval-source-map",
    devServer: {
        static: {
            directory: path.join(__dirname),
        },
        hot: true,
    },
    plugins: [
        new CopyWebpackPlugin({patterns: ['index.html', 'audio-worklet.js']}),
    ]
});
