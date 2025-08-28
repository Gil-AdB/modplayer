const CopyWebpackPlugin = require("copy-webpack-plugin");
const path = require('path');
const MiniCssExtractPlugin = require('mini-css-extract-plugin');

module.exports = {
    module: {
        rules: [
            {
                test: /\.(css)$/,
                use: ['style-loader', 'css-loader'],
            },
            {
                test: /\.(png|jpe?g|gif)$/i,
                loader: 'file-loader',
                options: {
                    outputPath: 'images',
                },
            },
        ],
    },
    experiments: {
        syncWebAssembly: true,
        // asyncWebAssembly: true,
    },
    entry: {
        bootstrap: "./bootstrap.js",
    },
    output: {
        path: path.resolve(__dirname, "dist"),
        filename: "bootstrap.js",
    },
   mode: "production",
   plugins: [
       new CopyWebpackPlugin({patterns: ['index.html']}),
       new MiniCssExtractPlugin()
   ]
    // mode: "production",
    // plugins: [
    //     new CopyWebpackPlugin(['index.html']),
    //     new MiniCssExtractPlugin()
    // ]
};
