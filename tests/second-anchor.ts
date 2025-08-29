import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import { ZooeyGo } from "../target/types/zooey_go";
import { TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { SystemProgram, PublicKey, TransactionMessage, VersionedTransaction, Connection, ComputeBudgetProgram } from "@solana/web3.js";
import { getAssociatedTokenAddressSync } from "@solana/spl-token";

/**
 * 🎉 DLMM池自动解析功能
 * 
 * 现在你只需要提供DLMM池地址，系统会自动：
 * 1. 从链上读取池的active_id
 * 2. 计算bin array indices (-1, 0, +1)
 * 3. 派生所有需要的PDA账户：
 *    - Oracle
 *    - EventAuthority  
 *    - ReserveX, ReserveY
 *    - BinArray -1, 0, +1
 * 
 * 使用方法：
 * const dlmmAccounts = await getDlmmPoolAccounts(
 *   connection,
 *   poolAddress,
 *   tokenXMint,
 *   tokenYMint, 
 *   dlmmProgramId
 * );
 */

// DLMM helper functions
const MAX_BIN_ARRAY_SIZE = new BN(70);

function binIdToBinArrayIndex(binId: BN): BN {
  const { div: idx, mod } = binId.divmod(MAX_BIN_ARRAY_SIZE);
  return binId.isNeg() && !mod.isZero() ? idx.sub(new BN(1)) : idx;
}

function deriveBinArray(lbPair: PublicKey, index: BN, programId: PublicKey) {
  let binArrayBytes: Uint8Array;
  if (index.isNeg()) {
    binArrayBytes = new Uint8Array(
      index.toTwos(64).toArrayLike(Buffer, "le", 8)
    );
  } else {
    binArrayBytes = new Uint8Array(index.toArrayLike(Buffer, "le", 8));
  }
  return PublicKey.findProgramAddressSync(
    [Buffer.from("bin_array"), lbPair.toBytes(), binArrayBytes],
    programId
  );
}

function deriveOracle(lbPair: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("oracle"), lbPair.toBytes()],
    programId
  );
}

function deriveReserve(token: PublicKey, lbPair: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [lbPair.toBuffer(), token.toBuffer()],
    programId
  );
}

function deriveEventAuthority(programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("__event_authority")],
    programId
  );
}

// 自动解析DLMM池账户的函数
async function getDlmmPoolAccounts(
  connection: Connection,
  poolAddress: PublicKey,
  programId: PublicKey
) {
  try {
    // 直接读取账户数据
    const accountInfo = await connection.getAccountInfo(poolAddress);
    if (!accountInfo) {
      throw new Error(`Pool account not found: ${poolAddress.toString()}`);
    }

    // 从账户数据中解析activeId (位置是固定的，在账户数据的特定偏移处)
    // 根据DLMM程序结构，activeId位于偏移量8+32+32+1+2+1 = 76字节处
    const data = accountInfo.data;
    const activeIdBytes = data.slice(76, 80); // i32 = 4字节
    const activeIdValue = activeIdBytes.readInt32LE(0);
    const activeId = new BN(activeIdValue);
    const tokenXMint = new PublicKey(data.slice(88, 120));
    const tokenYMint = new PublicKey(data.slice(120, 152));


    console.log(`Pool ${poolAddress.toString()} activeId: ${activeId.toString()}`);

    // 计算bin array indices
    const activeBinArrayIndex = binIdToBinArrayIndex(activeId);
    const binArrayMinus1Index = activeBinArrayIndex.sub(new BN(1));
    const binArrayPlus1Index = activeBinArrayIndex.add(new BN(1));

    // 派生所有需要的账户
    const [oracle] = deriveOracle(poolAddress, programId);
    const [eventAuthority] = deriveEventAuthority(programId);
    const [reserveX] = deriveReserve(tokenXMint, poolAddress, programId);
    const [reserveY] = deriveReserve(tokenYMint, poolAddress, programId);
    const [binArrayMinus1] = deriveBinArray(poolAddress, binArrayMinus1Index, programId);
    const [binArray0] = deriveBinArray(poolAddress, activeBinArrayIndex, programId);
    const [binArrayPlus1] = deriveBinArray(poolAddress, binArrayPlus1Index, programId);

    console.log(`Oracle: ${oracle.toString()}`);
    console.log(`EventAuthority: ${eventAuthority.toString()}`);
    console.log(`ReserveX: ${reserveX.toString()}`);
    console.log(`ReserveY: ${reserveY.toString()}`);
    console.log(`BinArray -1: ${binArrayMinus1.toString()}`);
    console.log(`BinArray 0: ${binArray0.toString()}`);
    console.log(`BinArray +1: ${binArrayPlus1.toString()}`);

    return {
      oracle,
      eventAuthority,
      reserveX,
      reserveY,
      binArrayMinus1,
      binArray0,
      binArrayPlus1,
      activeId: activeId.toNumber()
    };
  } catch (error) {
    console.error(`Failed to fetch pool data for ${poolAddress.toString()}:`, error);
    throw error;
  }
}



describe("second-anchor", () => {
  // Configure the client to use the local cluster.
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.zooeyGo as Program<ZooeyGo>;

  it("Is initialized!", async () => {
    // Add your test here.
    const provider = anchor.getProvider();
    const connection = new Connection(provider.connection.rpcEndpoint);

    const wsolMint = new PublicKey('So11111111111111111111111111111111111111112');
    const wsolTokenAccount = new PublicKey('GoF2hQNozCm2fB9HXDR7hFeFs4S3Pc15iF9m9oSC5wKF');
    const dlmmPoolProgramId = new PublicKey('LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo');
    const cpmmPoolProgramId = new PublicKey('CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C');

    // Token 2
    const tokenMint2 = new PublicKey('G8RBkZTf74Fn7gTFesTf8nztFujZngTqmUzvcX7VBAGS');
    const tokenMint2TokenAccount = getAssociatedTokenAddressSync(tokenMint2, provider.publicKey);

    // dammv2
    const dammv2PoolProgramId = new PublicKey('cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG');
    const dammv2EventAuthority = new PublicKey('3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet');
    const dammv2PoolAuthority = new PublicKey('HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC');
    const dammv2Pool = new PublicKey('A3c4ZYzMAuNy8Xv1eAhq4Q7Jg63x61oLnj9AxffQX6bj');
    const dammv2PoolTokenAVault = new PublicKey('GBnZBvbp8s7EDBn3UFoRNoyNJd9Zv8WV7DY1kdUny2XV');
    const dammv2PoolTokenBVault = new PublicKey('6YrhYaj9VouAnBUETxjERYdRammfqFxg6a25ApuWZE7T');
    
 
    //PUMP 
    const pumpPoolProgramId = new PublicKey('pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA');
    const pumpPool = new PublicKey('4F13bWhNZKASiwMsGrKi8JJEfhcTaxb19qQuGo7pXRr2');
   
    const pumpPoolGlobalConfig = new PublicKey('ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw');
    const pumpPoolEventAuthority = new PublicKey('GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR');
    const pumpPoolCoinCreatorVault = new PublicKey('CrT3U5kV7Bnp7KiqiFE3LivDZhp3uZPgUHCYSQkLJoco');
    const pumpPoolCoinCreatorVaultAuthority = new PublicKey('GQFxSthZdV8znEnC3u37iHXoLUUGAEXghNxPtwqjtYYd');
    const pumpPoolPumpFeeWallet = new PublicKey('9rPYyANsfQZw3DnDmKE3YCQF5E8oD89UXoHn9JFEhJUz');
    const pumpPoolPumpFeeWalletATA = new PublicKey('Bvtgim23rfocUzxVX9j9QFxTbBnH8JZxnaGLCEkXvjKS');

    const pumpPoolGlobalVolAccumulator = new PublicKey('C2aFPdENg4A2HQsmrd5rTw5TaYBX5Ku887cWjbFKtZpw');
    const pumpPoolUserVolAccumulator = new PublicKey('4s1EwU5NhpXJQMn5xT1HD5p7cv8YW9cgq5jKnZc6XbvR');
    const pumpPoolSystemProgram = new PublicKey('11111111111111111111111111111111');
    const pumpPoolAssociatedTokenProgram = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
    const pumpPoolBaseVault = new PublicKey('277HkrvaNbVx1wv7VgwgHTxYCQ6nJgLuRwXwR7s7BYUH');
    const pumpPoolQuoteVault = new PublicKey('2HUASL5pvE8kkCaDrqbXPNDV8i4SXtts7K1QEbDG37Pt');



    const dlmmPool3 = new PublicKey('9kLiRFUnPG7XMr2pQuqKA8uy3sNZ1d6WPG1rfTkMng6M');
    const dlmmAccounts3 = await getDlmmPoolAccounts(
      connection,
      dlmmPool3,
      dlmmPoolProgramId
    );

    const dlmmPool4 = new PublicKey('3EqgDzNZwkUt8nocNE3Xb9iADGRDt3h4nEGhw251RJL6');
    const dlmmAccounts4 = await getDlmmPoolAccounts(
      connection,
      dlmmPool4,
      dlmmPoolProgramId
    );

    const dlmmPool5 = new PublicKey('5oX5DXCSi6TX1NcrRBn1N5yfiR1NYdbXexxtXCtL4Rk8');
    const dlmmAccounts5 = await getDlmmPoolAccounts(
      connection,
      dlmmPool5,
      dlmmPoolProgramId
    );

    const dlmmPool6  = new PublicKey('2foBQHa7gbb5koSazbzeTEP789aUtpSotjHUFL7Y4LPt');
    const dlmmAccounts6 = await getDlmmPoolAccounts(
      connection,
      dlmmPool6,
      dlmmPoolProgramId
    );

    console.log('✅ DLMM池账户解析完成!');
    // console.log('📊 解析结果对比:');
    // console.log('  自动计算的Oracle:', dlmmAccounts3.oracle.toString());
    // console.log('  自动计算的ReserveX:', dlmmAccounts3.reserveX.toString());
    // console.log('  自动计算的ReserveY:', dlmmAccounts3.reserveY.toString());
    // console.log('  自动计算的BinArray-1:', dlmmAccounts3.binArrayMinus1.toString());
    // console.log('  自动计算的BinArray0:', dlmmAccounts3.binArray0.toString());
    // console.log('  自动计算的BinArray+1:', dlmmAccounts3.binArrayPlus1.toString());
    // console.log('  Active ID:', dlmmAccounts3.activeId);

    console.log('--------------------------------------------------------------------------------------------------------------------------------');


    

    const tokenMint3 = new PublicKey('7iGEQS6YdhhjRFtDkAQem58knuKimnoHGJ2pnoijBAGS');
    const tokenMint3TokenAccount = getAssociatedTokenAddressSync(tokenMint3, provider.publicKey);

    // dammv2池
    // const cpmmAuthority = new PublicKey('3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet');
    // const cpmmConfig = new PublicKey('D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2');
    // const cpmmObservationState = new PublicKey('9woVxF3LRkYU6hHzfSHcPMo5wfJs9Rpofc1JHgnQDBNv');
    // const cpmmPool = new PublicKey('BSh8SjXvauDvNRAzsVGibW1kB8eaaXWJYnf36SC9T7HC');
    // const cpmmTokenAVault = new PublicKey('136CHHCWu8zk1jg2h3DUZZt756mNqddSyNRmyokLTqKy');
    // const cpmmTokenBVault = new PublicKey('GWh7KHDfWC96ZRpWrXicznW7wzKcqfTsfa1X5jDnQimb');
    const dammv2PoolProgramId_2 = new PublicKey('cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG');
    const dammv2EventAuthority_2 = new PublicKey('3rmHSu74h1ZcmAisVcWerTCiRDQbUrBKmcwptYGjHfet');
    const dammv2PoolAuthority_2 = new PublicKey('HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC');
    const dammv2Pool_2 = new PublicKey('9KgyYoNFpMTxCRKVKXRmp8oqFF6rNLEFkNJbgxXYNgE');
    const dammv2PoolTokenAVault_2 = new PublicKey('2YquvcnzVxuHwDv56pviD8hjJa7du8XbmbDw19pyju72');
    const dammv2PoolTokenBVault_2 = new PublicKey('6u7EbawNzvKz3Uy3cBR2Qk5fzmnV8G6myeRP3RzCHwrr');



    // DLMM池 - 自动解析所有账户
    const dlmmPool2_4 = new PublicKey('5NazC6rkgnaBnhcTSN1LuEmAQDQp3cAjbuzQQnxdFKvH');
    const dlmmAccounts2_4 = await getDlmmPoolAccounts(
      connection,
      dlmmPool2_4,
      dlmmPoolProgramId
    );
   


    const tx = await program.methods.zooey(1, 0).accounts({
      payer: provider.publicKey,
      wsolMint: wsolMint,
      wsolTokenAccount: wsolTokenAccount,
      // systemProgram: SystemProgram.programId, 
      tokenProgram: TOKEN_PROGRAM_ID,
      // associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    })
      .remainingAccounts([
        // Token组信息
        { pubkey: tokenMint2, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: tokenMint2TokenAccount, isSigner: false, isWritable: true },

        { pubkey: dlmmPoolProgramId, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts3.eventAuthority, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts3.oracle, isSigner: false, isWritable: true },
        { pubkey: dlmmPool3, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts3.reserveX, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts3.reserveY, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts3.binArrayMinus1, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts3.binArray0, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts3.binArrayPlus1, isSigner: false, isWritable: true },

        // // dammv2池账户
        { pubkey: dammv2PoolProgramId, isSigner: false, isWritable: false },
        { pubkey: dammv2EventAuthority, isSigner: false, isWritable: false },
        { pubkey: dammv2PoolAuthority, isSigner: false, isWritable: false },
        { pubkey: dammv2Pool, isSigner: false, isWritable: true },
        { pubkey: dammv2PoolTokenAVault, isSigner: false, isWritable: true },
        { pubkey: dammv2PoolTokenBVault, isSigner: false, isWritable: true },
    
        // PUMP池账户
        // { pubkey: pumpPoolProgramId, isSigner: false, isWritable: false },
        // { pubkey: pumpPool, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolGlobalConfig, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolEventAuthority, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolCoinCreatorVault, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolCoinCreatorVaultAuthority, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolPumpFeeWallet, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolPumpFeeWalletATA, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolGlobalVolAccumulator, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolUserVolAccumulator, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolSystemProgram, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolAssociatedTokenProgram, isSigner: false, isWritable: false },
        // { pubkey: pumpPoolBaseVault, isSigner: false, isWritable: true },
        // { pubkey: pumpPoolQuoteVault, isSigner: false, isWritable: true },

        // DLMM池账户
        
       
        { pubkey: dlmmPoolProgramId, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts4.eventAuthority, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts4.oracle, isSigner: false, isWritable: true },
        { pubkey: dlmmPool4, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts4.reserveX, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts4.reserveY, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts4.binArrayMinus1, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts4.binArray0, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts4.binArrayPlus1, isSigner: false, isWritable: true },
 
       
        { pubkey: dlmmPoolProgramId, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts5.eventAuthority, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts5.oracle, isSigner: false, isWritable: true },
        { pubkey: dlmmPool5, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts5.reserveX, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts5.reserveY, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts5.binArrayMinus1, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts5.binArray0, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts5.binArrayPlus1, isSigner: false, isWritable: true },
 
     



      

        // // //tokenMint3
        { pubkey: tokenMint3, isSigner: false, isWritable: false },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: tokenMint3TokenAccount, isSigner: false, isWritable: true },

        // CPMM池账户 - 使用自动解析的账户
        { pubkey: dammv2PoolProgramId_2, isSigner: false, isWritable: false },
        { pubkey: dammv2EventAuthority_2, isSigner: false, isWritable: false },
        { pubkey: dammv2PoolAuthority_2, isSigner: false, isWritable: false },
        { pubkey: dammv2Pool_2, isSigner: false, isWritable: true },
        { pubkey: dammv2PoolTokenAVault_2, isSigner: false, isWritable: true },
        { pubkey: dammv2PoolTokenBVault_2, isSigner: false, isWritable: true },

        { pubkey: dlmmPoolProgramId, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts2_4.eventAuthority, isSigner: false, isWritable: false },
        { pubkey: dlmmAccounts2_4.oracle, isSigner: false, isWritable: true },
        { pubkey: dlmmPool2_4, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts2_4.reserveX, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts2_4.reserveY, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts2_4.binArrayMinus1, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts2_4.binArray0, isSigner: false, isWritable: true },
        { pubkey: dlmmAccounts2_4.binArrayPlus1, isSigner: false, isWritable: true },
 

      ]).instruction()


    let count = 0;

 
    const lookupTableAccount8 = await connection.getAddressLookupTable(new PublicKey('4sKLJ1Qoudh8PJyqBeuKocYdsZvxTcRShUt9aKqwhgvC'));
    const lookupTableAccount9 = await connection.getAddressLookupTable(new PublicKey('9VmnnZQLCAVyNkrGoKmYojBkv3KEQFFfhnpN65fK4qDM'));
    const lookupTableAccount10 = await connection.getAddressLookupTable(new PublicKey('H8s6DCRVbyUUDULb9hKbEvt9Ed3D4ifWUeujEJG57TJG'));
    const lookupTableAccount11 = await connection.getAddressLookupTable(new PublicKey('FLVnikJRcVF8qtCATajymFZMkjbTFmeGsE5QaRnUkjHo'));
    const lookupTableAccount12 = await connection.getAddressLookupTable(new PublicKey('2x53gwFo59Spyu77Ns3cB6AyC9ZDZsZLG4ZvQXeKy8yD'));
    const lookupTableAccount13 = await connection.getAddressLookupTable(new PublicKey('BdK993bMGz7G2sJEiyDwcbT53b262GqpS3Up4f1QRSoM'));
    const lookupTableAccount14 = await connection.getAddressLookupTable(new PublicKey('6fLdHzPQooFcxyEvnZtr6jfbtcr5Wg1rTpQVYZEqKQ58'));
    
    // 检查lookup table账户状态
 
    while (count < 30000) {
      const latestBlockhash = await connection.getLatestBlockhash();
      const computeBudgetInstruction = ComputeBudgetProgram.setComputeUnitLimit({ units: 440_000 });
      const computeBudgetPrice = ComputeBudgetProgram.setComputeUnitPrice({ microLamports: 22000 });

      // 过滤出存在的lookup table账户
      const validLookupTables = [
        lookupTableAccount8.value,
        lookupTableAccount9.value,
        lookupTableAccount10.value,
        lookupTableAccount11.value,
        lookupTableAccount12.value,
        lookupTableAccount13.value,
        lookupTableAccount14.value
      ].filter(table => table !== null);
      
      // console.log(`Using ${validLookupTables.length} valid lookup tables`);
      
      // 如果没有有效的lookup tables，使用普通交易
      let messageV0;
      if (validLookupTables.length === 0) {
        console.log('No valid lookup tables found, using regular transaction');
        messageV0 = new TransactionMessage({
          payerKey: provider.publicKey,
          recentBlockhash: latestBlockhash.blockhash,
          instructions: [computeBudgetInstruction, computeBudgetPrice, tx],
        }).compileToV0Message();
      } else {
        messageV0 = new TransactionMessage({
          payerKey: provider.publicKey,
          recentBlockhash: latestBlockhash.blockhash,
          instructions: [computeBudgetInstruction, computeBudgetPrice, tx],
        }).compileToV0Message(validLookupTables);
      }

      const v0Tx = new VersionedTransaction(messageV0);


      // 用钱包签名（自动适配 Anchor Provider）
      const signedTx = await provider.wallet.signTransaction(v0Tx);

      try {
        const txSig = await connection.sendTransaction(signedTx, {
          skipPreflight: true,
        });
        console.log("Your V0 transaction signature:", txSig);
      } catch (e) {
        console.log(e);
        console.log('--------------------------------');
      }

      count++;
      await new Promise(resolve => setTimeout(resolve, 500));
    }

  });
});
